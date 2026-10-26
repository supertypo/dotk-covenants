# The dotk market escrow

The sale escrow of the dotk market, which sells `.k` names of the [dotk.name](https://dotk.name)
registry on Kaspa, and a verifier that proves the listings that the market serves.

`sil/dotk_listing.v2.sil` holds one listed name until a buyer pays the price, the seller cancels,
or the listing's window passes. `sil/dotk_listing.v2.json` is the source compiled. Every listing
runs this template with its own terms in the state. [GUIDE.md](GUIDE.md) tells a venue how to open
listings, and how to buy, cancel and expire them.

## Verify the listings

Install Rust with [rustup](https://rustup.rs), then run:

```sh
git clone https://github.com/supertypo/dotk-covenants
cd dotk-covenants/market
./verify.sh
```

The verifier builds on the registry verifier in [`../registry`](../registry), so it needs the
whole repository. It proves four facts:

1. The registry runs on its contracts, as `../registry` proves it. This gives the registry's
   covenant id and its deed template, which every escrow state names.
2. The escrow source compiles to the published template, whose hash this repository pins.
3. The terms of every listing rebuild to its escrow. The terms are ones that a buy can pass, and
   every address and hash that the market serves for the listing is the one that the terms give.
4. Every listing's deed sits in its escrow on chain, and every reserve that the market serves
   exists with the DAA score that the expiry counts from.

The escrow's hash commits to every term, so a deed that the node shows at that escrow proves the
served terms exactly. The verifier reads the listings from the dotk market API and checks each
one against a Kaspa node. You trust only the SilverScript compiler, the pinned rusty-kaspa crates
and the node.

The proof covers the listings that the market serves. It cannot show that the market serves every
listing on chain. A listing of another escrow version is counted, but not proven. Before each
step, the verifier explains the check and waits for a key press. Its exit code is 0 for proven
listings, 1 for a failed step and 2 for a run that did not finish.

| Option | Effect |
|---|---|
| `-s <node>` | Use this Kaspa node (wRPC Borsh), for example `ws://127.0.0.1:17110`, not the public resolver |
| `--network <name>` | `mainnet`, the default, or `testnet-10` |
| `--name <name>` | Prove the listing of this one name |
| `--api <url>` | Read the registry from this API, by default `https://api.dotk.name/v1` |
| `--market <url>` | Read the listings from this API, by default `https://market-api.dotk.name/v1` |
| `-q` | Print only the name and result of each check |

`cargo test` shows that the published artifact is the source compiled, that the state encoder
writes what the compiler writes, and that every tampered or wrong answer from the API or the node
fails the proof.
