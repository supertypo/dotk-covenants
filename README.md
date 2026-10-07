# dotk-covenants

The covenant contracts of [dotk.name](https://dotk.name), the `.k` name registry on Kaspa, and a
verifier that proves that the mainnet registry runs on them.

`sil/dotk_gap.sil` holds the open intervals of unregistered names, and `sil/dotk_deed.sil` holds one
registered name and its owner. The [developers page](https://dotk.name/developers) explains the
design. The rest of the project is at [supertypo/dotk](https://github.com/supertypo/dotk).

## Verify the mainnet registry

Install Rust with [rustup](https://rustup.rs), then run:

```sh
git clone https://github.com/supertypo/dotk-covenants
cd dotk-covenants
./verify.sh
```

The verifier compiles both contracts with the registry's parameters and proves four facts:

1. The contracts compile to the deployed bytecode.
2. The pinned covenant id of the mainnet registry commits to that bytecode.
3. The gaps on chain cover the whole keyspace, so the registered keys are exactly the ones that
   the dotk API lists.
4. Every active deed on chain has the name and owner that the API lists, and every pending deed
   its key and claim.

It reads the claims from the dotk API and checks each one against a Kaspa node. You trust only the
SilverScript compiler, the pinned rusty-kaspa crates and the node. Before each step, the verifier
explains the check and waits for a key press. Its exit code is 0 for a proven registry, 1 for a
failed step and 2 for a run that did not finish.

| Option | Effect |
|---|---|
| `-s <node>` | Use this Kaspa node (wRPC Borsh), for example `ws://127.0.0.1:17110`, not the public resolver |
| `--api <url>` | Read the claims from this API, by default `https://api.dotk.name/v1` |
| `-q` | Print only the name and result of each check |

The proof ties the live registry to these exact sources, so a review of `sil/` is a review of what
runs on mainnet. `cargo test` shows that every tampered or wrong answer from the API or the node
fails the proof.

## License

MIT
