mod node;

use std::collections::HashSet;
use std::io::{IsTerminal, Write};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use crossterm::style::{Color, Stylize};
use dotk_covenants::api;
use dotk_covenants::compile::Templates;
use dotk_covenants::verify::{self, Probe};
use kaspa_addresses::Address;
use kaspa_rpc_core::RpcHash;
use node::Node;

const DEFAULT_API: &str = "https://api.dotk.name/v1";
/// The registry can change between the snapshot and the lookups, which a fresh snapshot settles.
const ATTEMPTS: usize = 3;
const RETRY_DELAY: Duration = Duration::from_secs(15);

const USAGE: &str = "\
Usage: dotk-verify [-q] [-s <node>] [--api <url>]

Proves that the dotk.name registry on Kaspa mainnet is produced by the contracts in sil/
and the registry's parameters.

  -s <node>    Use this one Kaspa node, for example ws://127.0.0.1:17110 (wRPC Borsh).
               Without it, the public resolver picks a mainnet node.
  --api <url>  The dotk API to read the claims from (default https://api.dotk.name/v1).
  -q           Print only each check's name and result, without explanations or pauses.
  -h           Show this help.

Exit codes: 0 proven, 1 not proven, 2 could not run (for example, no node answered).";

struct Args {
    api: String,
    node: Option<String>,
    quiet: bool,
}

fn parse_args() -> Result<Option<Args>> {
    let mut args = Args { api: DEFAULT_API.to_string(), node: None, quiet: false };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-s" => args.node = Some(it.next().ok_or_else(|| anyhow!("-s needs a node URL"))?),
            "--api" => args.api = it.next().ok_or_else(|| anyhow!("--api needs a URL"))?.trim_end_matches('/').to_string(),
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
        name: "The contracts compile to the deployed bytecode",
        explanation: "\
The verifier compiles sil/dotk_gap.sil and sil/dotk_deed.sil with silverscript v1.0.0,
with the registry's parameters (fees, bond, deposit, gap value, eviction time and
the devfund script) built into the bytecode, exactly as the deployment did.

It then compares the result byte for byte with the bytecode and template hashes
that the API publishes for the deployment. The parameters come from the API, but
they are not trusted: wrong parameters give different bytecode. If the API also
published bytecode to match them, the covenant id of step 2 would not match.",
    },
    Step {
        number: 2,
        name: "The covenant id commits to this bytecode",
        explanation: "\
Every registry UTXO carries one covenant id (KIP-20). Kaspa consensus computes it
once, at genesis, as a hash of the funding outpoint and of the genesis output: its
value and its script. The genesis output was one gap that covers the whole keyspace.

The verifier rebuilds that gap's script from the bytecode of step 1 and computes the
covenant id offline. It then makes sure that the id equals the mainnet registry's id,
which this repository pins, so no API can point the proof at another covenant. A
UTXO with this id can only exist if it descends from that exact genesis gap, through
spends that the contracts allowed.",
    },
    Step {
        number: 3,
        name: "The gaps on chain partition the whole keyspace",
        explanation: "\
Each name occupies one key, blake3(name). The registry is a set of gaps (open
intervals of free keys) and deeds (one per registered key). The contracts keep this
invariant: every key is inside exactly one gap or is exactly one deed's key.

The verifier reads the list of registered keys from the API snapshot and derives the
gap between each pair of neighbors, from the lowest to the highest possible key. It
computes each gap's address from the bytecode and asks the Kaspa node whether that
address holds a UTXO with the registry's covenant id. If every gap exists, the gaps
cover the keyspace end to end, so the registered keys are exactly the snapshot's keys.
A key that the API left out, or added, leaves a gap that does not exist on chain.",
    },
    Step {
        number: 4,
        name: "Every deed on chain matches its owner",
        explanation: "\
For each registered name, the snapshot claims a state: the name and its owner, or
the claim of a pending registration. The verifier makes sure that blake3(name) gives
the deed's key, builds the deed's state, computes its address from the bytecode, and
asks the node whether that address holds a UTXO with the registry's covenant id.

A wrong owner or a wrong name gives an address with no UTXO, so this proves who owns
each name. A pending deed has not revealed its name yet, so for it this proves the key
and the claim. A deed that the API lists without an owner has no state to rebuild, so it
fails this step.",
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

/// A snapshot that its own content refutes, which fails the step rather than the run.
#[derive(Debug)]
struct Refuted(anyhow::Error);

impl std::fmt::Display for Refuted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#}", self.0)
    }
}

impl std::error::Error for Refuted {}

struct OnChain {
    probe: Probe,
    gaps_found: HashSet<Address>,
    deeds_found: Option<HashSet<Address>>,
}

impl OnChain {
    fn missing(&self) -> usize {
        self.probe.missing_gaps(&self.gaps_found) + self.deeds_found.as_ref().map_or(0, |f| self.probe.missing_deeds(f).len())
    }

    fn check_gaps(&self) -> Result<String> {
        self.probe.check_gaps(&self.gaps_found)
    }

    fn check_deeds(&self) -> Result<String> {
        self.check_gaps().map_err(|e| e.context("the snapshot that step 4 read"))?;
        self.probe.check_deeds(self.deeds_found.as_ref().ok_or_else(|| anyhow!("the deeds were not looked up"))?)
    }
}

struct Chain<'a> {
    ui: &'a Ui,
    http: &'a reqwest::Client,
    api: &'a str,
    templates: &'a Templates,
    id: RpcHash,
}

impl Chain<'_> {
    /// A fresh snapshot, with its gaps looked up, and its deeds too if `deeds` is set.
    async fn read(&self, node: &Node, deeds: bool) -> Result<OnChain> {
        let snapshot = api::snapshot(self.http, self.api).await?;
        let probe = verify::probe(&snapshot, self.templates, self.id).map_err(|e| anyhow::Error::new(Refuted(e)))?;
        let gaps_found = node.holding(&probe.gaps, self.id).await?;
        let deeds_found = if deeds {
            let addresses: Vec<Address> = probe.deeds.iter().map(|(_, a)| a.clone()).collect();
            Some(node.holding(&addresses, self.id).await?)
        } else {
            None
        };
        Ok(OnChain { probe, gaps_found, deeds_found })
    }

    async fn read_settled(&self, node: &Node, deeds: bool) -> Result<OnChain> {
        for attempt in 1..=ATTEMPTS {
            let current = self.read(node, deeds).await?;
            if current.missing() == 0 || attempt == ATTEMPTS {
                return Ok(current);
            }
            self.ui.warn(&format!(
                "{} UTXOs not found. A registration since the snapshot also causes this, so the verifier \
                 gets a fresh snapshot in {} s (retry {attempt} of {}).",
                current.missing(),
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

async fn run(args: Args) -> Result<bool> {
    let ui = Ui { quiet: args.quiet, color: colors_on(std::io::stdout().is_terminal()) };
    let http = reqwest::Client::builder().timeout(Duration::from_secs(120)).build()?;

    ui.info("dotk-covenants verifier: proves that the dotk.name registry on Kaspa mainnet is produced");
    ui.info("by the contracts in sil/ and the registry's parameters.\n");
    ui.info(&format!("Reading the deployment manifest from {}/genesis", args.api));
    let manifest = api::manifest(&http, &args.api).await?;
    if args.node.is_none() {
        let warning = "No node specified. A public node is used. It can be slow or unstable. See README.md";
        eprintln!("{}", paint(colors_on(std::io::stderr().is_terminal()), Color::Yellow, warning));
    }
    let mut node = Node::connect(args.node.as_deref()).await?;
    ui.info(&format!("Kaspa node: {} (kaspad {})", node.url, node.version));

    ui.announce(&STEPS[0]);
    let t = match verify::check_compile(&manifest) {
        Ok((t, detail)) => {
            ui.pass(&detail);
            t
        }
        Err(e) => {
            ui.fail(&e);
            return Ok(false);
        }
    };

    ui.announce(&STEPS[1]);
    let id = match verify::check_genesis(&manifest, &t) {
        Ok((id, detail)) => {
            ui.pass(&detail);
            id
        }
        Err(e) => {
            ui.fail(&e);
            return Ok(false);
        }
    };

    let chain = Chain { ui: &ui, http: &http, api: &args.api, templates: &t, id };

    // Steps 3 and 4 each read their own snapshot, so a long pause at a prompt changes nothing.
    ui.announce(&STEPS[2]);
    node = reconnected(&ui, node).await?;
    let gaps = match chain.read_settled(&node, false).await {
        Ok(gaps) => gaps,
        Err(e) if e.is::<Refuted>() => {
            ui.fail(&e);
            ui.skip(&STEPS[3]);
            node.disconnect().await;
            return Ok(false);
        }
        Err(e) => return Err(e),
    };
    let structure = gaps.check_gaps();
    match &structure {
        Ok(detail) => ui.pass(detail),
        Err(e) => ui.fail(e),
    }

    // Owners mean nothing without a proven key set.
    if structure.is_err() {
        ui.skip(&STEPS[3]);
        node.disconnect().await;
        return Ok(false);
    }

    ui.announce(&STEPS[3]);
    node = reconnected(&ui, node).await?;
    let deeds = match chain.read_settled(&node, true).await {
        Ok(settled) => settled.check_deeds(),
        Err(e) if e.is::<Refuted>() => Err(e),
        Err(e) => return Err(e),
    };
    match &deeds {
        Ok(detail) => ui.pass(detail),
        Err(e) => ui.fail(e),
    }
    node.disconnect().await;
    Ok(structure.is_ok() && deeds.is_ok())
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
        Ok(true) => {
            if !quiet {
                let result = "RESULT: PROVEN. The registry on mainnet is produced by these contracts and parameters.";
                println!("\n{}", paint(color, Color::Green, result));
            }
            ExitCode::SUCCESS
        }
        Ok(false) => {
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
