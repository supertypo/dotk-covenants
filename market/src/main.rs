mod node;

use std::io::{IsTerminal, Write};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use crossterm::style::{Color, Stylize};
use dotk_listing::api;
use dotk_listing::escrow::Registry;
use dotk_listing::verify::{self, Probe};
use dotk_registry::compile::Template;
use dotk_registry::verify::{MAINNET, Network, TESTNET_10};
use node::Node;

/// A listing can close between the read and the lookups, which a fresh read settles.
const ATTEMPTS: usize = 3;
const RETRY_DELAY: Duration = Duration::from_secs(15);

const USAGE: &str = "\
Usage: dotk-listing-verify [-q] [-s <node>] [--network <name>] [--name <name>] [--api <url>] [--market <url>]

Proves that the listings that the dotk market serves run on the escrow in sil/, with exactly
the terms that the market serves.

  -s <node>         Use this one Kaspa node, for example ws://127.0.0.1:17110 (wRPC Borsh).
                    Without it, the public resolver picks a node.
  --network <name>  mainnet (the default) or testnet-10.
  --name <name>     Prove the listing of this one name, not every listing.
  --api <url>       The dotk API to read the registry from (default https://api.dotk.name/v1).
  --market <url>    The dotk market API to read the listings from
                    (default https://market-api.dotk.name/v1).
  -q                Print only each check's name and result, without explanations or pauses.
  -h                Show this help.

Exit codes: 0 proven, 1 not proven, 2 could not run (for example, no node answered).";

struct Args {
    network: &'static Network,
    api: Option<String>,
    market: Option<String>,
    name: Option<String>,
    node: Option<String>,
    quiet: bool,
}

impl Args {
    fn api(&self) -> String {
        self.api.clone().unwrap_or_else(|| {
            if self.network.name == TESTNET_10.name { "https://api-tn10.dotk.name/v1" } else { "https://api.dotk.name/v1" }.to_string()
        })
    }

    fn market(&self) -> String {
        self.market.clone().unwrap_or_else(|| {
            if self.network.name == TESTNET_10.name {
                "https://market-api-tn10.dotk.name/v1"
            } else {
                "https://market-api.dotk.name/v1"
            }
            .to_string()
        })
    }
}

fn url(it: &mut impl Iterator<Item = String>, flag: &str) -> Result<String> {
    Ok(it.next().ok_or_else(|| anyhow!("{flag} needs a URL"))?.trim_end_matches('/').to_string())
}

fn parse_args() -> Result<Option<Args>> {
    let mut args = Args { network: &MAINNET, api: None, market: None, name: None, node: None, quiet: false };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-s" => args.node = Some(it.next().ok_or_else(|| anyhow!("-s needs a node URL"))?),
            "--network" => {
                args.network = match it.next().as_deref() {
                    Some("mainnet") => &MAINNET,
                    Some("testnet-10") => &TESTNET_10,
                    other => bail!("--network takes mainnet or testnet-10, not {other:?}"),
                }
            }
            "--name" => args.name = Some(it.next().ok_or_else(|| anyhow!("--name needs a name"))?),
            "--api" => args.api = Some(url(&mut it, "--api")?),
            "--market" => args.market = Some(url(&mut it, "--market")?),
            "-q" => args.quiet = true,
            "-h" | "--help" => return Ok(None),
            other => bail!("unknown argument {other}\n\n{USAGE}"),
        }
    }
    Ok(Some(args))
}

struct Step {
    number: usize,
    name: &'static str,
    explanation: &'static str,
}

const STEPS: [Step; 4] = [
    Step {
        number: 1,
        name: "The registry runs on its contracts",
        explanation: "\
A listing holds a name's deed, and the escrow names the registry's covenant id and the
deed template in its state. The verifier proves both first, with the registry
verifier in ../registry: it compiles sil/dotk_gap.sil and sil/dotk_deed.sil with the
registry's parameters, compares the result with the deployed bytecode, and makes sure
that the genesis gap gives the registry's covenant id, which this repository pins.",
    },
    Step {
        number: 2,
        name: "The escrow compiles to the published template",
        explanation: "\
The verifier compiles sil/dotk_listing.v2.sil with silverscript v1.0.0 and compares the
bytes around its state with sil/dotk_listing.v2.json. Every listing's escrow is this
template with the listing's own state, so the template hash, which this repository
pins, names the code that every listing runs.",
    },
    Step {
        number: 3,
        name: "Every listing's terms rebuild to its escrow",
        explanation: "\
For each listing, the verifier reads the terms that the market serves: seller, price,
fee, window, venue and salt. It makes sure that the terms are ones a buy can pass, and
writes them into the escrow state with the registry of step 1. The hash of that script
is the escrow, the deed's owner. Every address and hash that the market serves for the
listing must be the one that the terms give.

A listing of another escrow version is counted, but not proven.",
    },
    Step {
        number: 4,
        name: "Every listing's deed sits in its escrow",
        explanation: "\
The deed of a listed name is owned by the escrow's hash. The verifier computes the
address of that deed and asks the Kaspa node whether it holds a UTXO with the
registry's covenant id. Only the escrow of exactly these terms can own it, so a wrong
term gives an address with no UTXO. It also makes sure that the reserve the market
serves exists, at the escrow, with the DAA score that the expiry counts from.",
    },
];

/// Waits for one key press. Without a terminal nobody can press one, so it returns at once.
fn wait_for_key() {
    use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
    use crossterm::terminal;

    if !std::io::stdin().is_terminal() {
        return;
    }
    print!("Press any key to continue ");
    let _ = std::io::stdout().flush();
    let key = terminal::enable_raw_mode().and_then(|()| {
        loop {
            if let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                break Ok(key);
            }
        }
    });
    let _ = terminal::disable_raw_mode();
    println!();
    // Raw mode delivers Ctrl+C as a key, so it stops the run here.
    if let Ok(key) = key
        && key.code == KeyCode::Char('c')
        && key.modifiers.contains(KeyModifiers::CONTROL)
    {
        std::process::exit(130);
    }
}

/// Colors only on a terminal, and never when NO_COLOR is set, so a log or a pipe stays plain text.
fn colors_on(terminal: bool) -> bool {
    terminal && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty())
}

fn paint(on: bool, color: Color, text: &str) -> String {
    if on { text.with(color).to_string() } else { text.to_string() }
}

struct Ui {
    quiet: bool,
    color: bool,
}

impl Ui {
    fn announce(&self, step: &Step) {
        if self.quiet {
            print!("[{}/4] {} ... ", step.number, step.name);
            let _ = std::io::stdout().flush();
            return;
        }
        println!("\n=== Step {} of 4: {} ===\n", step.number, step.name);
        println!("{}\n", step.explanation);
        wait_for_key();
        println!("Running...");
    }

    fn pass(&self, detail: &str) {
        if self.quiet {
            println!("{}", paint(self.color, Color::Green, "ok"));
        } else {
            println!("{} {detail}", paint(self.color, Color::Green, "PASS:"));
        }
    }

    /// A failed proof: the answer is wrong, not the run.
    fn fail(&self, e: &anyhow::Error) {
        if self.quiet {
            println!("{}", paint(self.color, Color::Red, &format!("FAIL\n  {e:#}")));
        } else {
            println!("{}", paint(self.color, Color::Red, &format!("FAIL: {e:#}")));
        }
    }

    /// A transient problem that the verifier works around, such as a retry or a new node.
    fn warn(&self, text: &str) {
        if !self.quiet {
            println!("{}", paint(self.color, Color::Yellow, text));
        }
    }

    fn skip(&self, step: &Step) {
        if self.quiet {
            println!("[{}/4] {} ... skipped", step.number, step.name);
        } else {
            println!("\n=== Step {} of 4: {} ===\n\nSkipped, because step {} failed.", step.number, step.name, step.number - 1);
        }
    }

    fn info(&self, text: &str) {
        if !self.quiet {
            println!("{text}");
        }
    }
}

/// An answer that its own content refutes, which fails the step rather than the run.
#[derive(Debug)]
struct Refuted(anyhow::Error);

impl std::fmt::Display for Refuted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#}", self.0)
    }
}

impl std::error::Error for Refuted {}

struct Chain<'a> {
    ui: &'a Ui,
    http: &'a reqwest::Client,
    args: &'a Args,
    template: &'a Template,
    registry: &'a Registry,
}

impl Chain<'_> {
    async fn served(&self) -> Result<Vec<api::Listing>> {
        let market = self.args.market();
        match &self.args.name {
            None => api::listings(self.http, &market).await,
            Some(name) => {
                let refuted = |e: anyhow::Error| anyhow::Error::new(Refuted(e));
                let listing = api::listing(self.http, &market, name)
                    .await?
                    .ok_or_else(|| refuted(anyhow!("the market serves no listing of {name}")))?;
                verify::check_named(&listing, name).map_err(refuted)?;
                Ok(vec![listing])
            }
        }
    }

    /// A fresh read of the listings, rebuilt, and with their addresses looked up if `node` is set.
    async fn read(&self, node: Option<&Node>) -> Result<(Probe, verify::Found)> {
        let served = self.served().await?;
        let probe =
            verify::probe(&served, self.template, self.registry, self.args.network).map_err(|e| anyhow::Error::new(Refuted(e)))?;
        let found = match node {
            Some(node) => verify::by_address(node.utxos_at(&probe.addresses()).await?),
            None => verify::Found::new(),
        };
        Ok((probe, found))
    }

    async fn read_settled(&self, node: &Node) -> Result<(Probe, verify::Found)> {
        for attempt in 1..=ATTEMPTS {
            let (probe, found) = self.read(Some(node)).await?;
            let missing = probe.missing(&found, self.registry).len();
            if missing == 0 || attempt == ATTEMPTS {
                return Ok((probe, found));
            }
            self.ui.warn(&format!(
                "{missing} listings not found on chain. A sale or a cancel since the read also causes this, so the \
                 verifier reads the listings again in {} s (retry {attempt} of {}).",
                RETRY_DELAY.as_secs(),
                ATTEMPTS - 1
            ));
            tokio::time::sleep(RETRY_DELAY).await;
        }
        unreachable!("the last attempt returns")
    }
}

/// The node again, or a replacement if it stopped answering during a pause.
async fn reconnected(ui: &Ui, node: Node) -> Result<Node> {
    let before = node.url.clone();
    let node = node.alive().await?;
    if node.url != before {
        ui.warn(&format!("The node stopped answering. Kaspa node: {} (kaspad {})", node.url, node.version));
    }
    Ok(node)
}

fn rebuilt(probe: &Probe) -> String {
    match probe.listings.len() {
        0 => "the market serves no listing of this escrow".to_string(),
        n => format!("all {n} listings rebuild to the escrow they name"),
    }
}

fn others(probe: &Probe) -> String {
    match probe.other_versions {
        0 => String::new(),
        n => format!("\n  {n} listings of another escrow version, which this repository does not cover"),
    }
}

/// The listings proven, and those of another version, or `None` for a failed proof.
async fn run(args: Args) -> Result<Option<(usize, usize)>> {
    let ui = Ui { quiet: args.quiet, color: colors_on(std::io::stdout().is_terminal()) };
    let http = reqwest::Client::builder().timeout(Duration::from_secs(120)).build()?;

    ui.info(&format!("dotk.name listing verifier: proves that the listings that the dotk market serves on {}", args.network.name));
    ui.info("run on the escrow in sil/, with exactly the terms that the market serves.\n");
    ui.info(&format!("Reading the registry manifest from {}/genesis", args.api()));
    let manifest = dotk_registry::api::manifest(&http, &args.api()).await?;
    if args.node.is_none() {
        let warning = "No node specified. A public node is used. It can be slow or unstable. See README.md";
        eprintln!("{}", paint(colors_on(std::io::stderr().is_terminal()), Color::Yellow, warning));
    }
    let mut node = Node::connect(args.node.as_deref(), args.network).await?;
    ui.info(&format!("Kaspa node: {} (kaspad {})", node.url, node.version));

    ui.announce(&STEPS[0]);
    let registry = match verify::check_registry(&manifest, args.network) {
        Ok((registry, detail)) => {
            ui.pass(&detail);
            registry
        }
        Err(e) => {
            ui.fail(&e);
            return Ok(None);
        }
    };

    ui.announce(&STEPS[1]);
    let template = match verify::check_escrow(&registry) {
        Ok((template, detail)) => {
            ui.pass(&detail);
            template
        }
        Err(e) => {
            ui.fail(&e);
            return Ok(None);
        }
    };

    let chain = Chain { ui: &ui, http: &http, args: &args, template: &template, registry: &registry };

    // Steps 3 and 4 each read the listings again, so a long pause at a prompt changes nothing.
    ui.announce(&STEPS[2]);
    match chain.read(None).await {
        Ok((probe, _)) => ui.pass(&format!("{}{}", rebuilt(&probe), others(&probe))),
        Err(e) if e.is::<Refuted>() => {
            ui.fail(&e);
            ui.skip(&STEPS[3]);
            node.disconnect().await;
            return Ok(None);
        }
        Err(e) => return Err(e),
    }

    ui.announce(&STEPS[3]);
    node = reconnected(&ui, node).await?;
    let on_chain = match chain.read_settled(&node).await {
        Ok((probe, found)) => {
            probe.check_chain(&found, &registry).map(|detail| (detail, (probe.listings.len(), probe.other_versions)))
        }
        Err(e) if e.is::<Refuted>() => Err(e),
        Err(e) => return Err(e),
    };
    match &on_chain {
        Ok((detail, _)) => ui.pass(detail),
        Err(e) => ui.fail(e),
    }
    node.disconnect().await;
    Ok(on_chain.ok().map(|(_, n)| n))
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(Some(args)) => args,
        Ok(None) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("{e:#}");
            return ExitCode::from(2);
        }
    };
    let quiet = args.quiet;
    let color = colors_on(std::io::stdout().is_terminal());
    match run(args).await {
        Ok(Some((proven, others))) => {
            if !quiet {
                let mut result = match proven {
                    0 => "RESULT: PROVEN. The market serves no listing of this escrow, so no listing claims it falsely.".to_string(),
                    n => format!("RESULT: PROVEN. All {n} listings of this escrow run on it, with the terms that the market serves."),
                };
                if others > 0 {
                    result.push_str(&format!(" {others} listings of another escrow version are not proven."));
                }
                println!("\n{}", paint(color, Color::Green, &result));
            }
            ExitCode::SUCCESS
        }
        Ok(None) => {
            if !quiet {
                println!("\n{}", paint(color, Color::Red, "RESULT: NOT PROVEN. See the failed step above."));
            }
            ExitCode::FAILURE
        }
        Err(e) => {
            // In -q mode, a step's name waits for its result on the current line.
            println!();
            eprintln!("{}", paint(colors_on(std::io::stderr().is_terminal()), Color::Yellow, &format!("error: {e:#}")));
            ExitCode::from(2)
        }
    }
}
