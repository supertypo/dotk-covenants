# Listing .k names on the dotk market

This guide tells a partner venue how to open listings of `.k` names that the dotk market finds,
serves and sells. A venue is the party that receives the fee of a sale. The escrow is in `sil/`:

- `sil/dotk_listing.v2.sil`, the source of the sale escrow.
- `sil/dotk_listing.v2.json`, the compiled escrow, with the template hash
  `cfc5bc3f94cb3c8d80b30091adaa126ce159e076e72aa13952ac3bde7de179e4`.

The registry covenants are in [`../registry`](../registry), and
[supertypo/dotk-core](https://github.com/supertypo/dotk-core) builds their transactions. The
escrow compiles with silverscript v1.0.0, and its signature scripts encode with
`silverscript-abi` at the same tag.

## How a listing works

A listing is a registry `transfer` of the seller's deed to a P2SH owner. The P2SH hash is the
blake2b-256 hash of one escrow script, which holds the terms of that one listing. The escrow has
three entrypoints:

- `buy`: anyone pays the price, and the name goes to the buyer.
- `cancel`: the seller signs, and the name goes back to the seller.
- `expire`: after the window, anyone returns the name to the seller.

The escrow pins every value in its state, so no party can change a term after the listing.

## Before you start

1. Send your venue address to dotk. The dotk market accepts a listing of any venue, but the
   dotk web app shows a known venue under its name, as `dotk/<your name>`.
2. Make sure that your venue address is a schnorr address or an ECDSA address. A P2SH address
   cannot be a venue.

## The terms

| Term | Rule |
|---|---|
| Price | What the buyer pays, in sompi. From 2 KAS to 80,000,000 KAS. The dotk market serves no listing above that ceiling. |
| Fee | Your cut, in sompi. At least 0.2 KAS (`20000000`), and at most the price divided by 10, rounded down. |
| Window | The DAA score age after which anyone can run `expire`. `0` means that the listing never expires. At most `4294967295`. |
| Venue | Your key, as a scheme byte and a 32-byte key. |
| Seller | The deed's owner before the listing. The seller must own the deed by a key: scheme `0x00`, `0x85` or `0x86`. |

The fee comes out of the price. At a buy, the seller receives the price less the fee, and the
0.5 KAS reserve. The buyer pays the price and the network fee only.

The script refuses a buy whose fee is outside the bounds. The seller can still cancel such a
listing, but nobody can buy it, and the dotk market does not show it.

The scheme byte of a schnorr key is `0x00`. An ECDSA key with an even y takes `0x86`, and one
with an odd y takes `0x85`. An ECDSA key is stored as its 32-byte x.

## Build the escrow script

The escrow script is the artifact's `bytecode` with its state span replaced. The state span starts
at byte 1 and is 251 bytes long (`state_span` in the artifact). Write the 13 fields below in this
order, each one as a data push:

| # | Field | Push |
|---|---|---|
| 1 | Seller scheme | `0x01` and 1 byte |
| 2 | Seller key | `0x20` and 32 bytes |
| 3 | Name key, `blake3(name)` | `0x20` and 32 bytes |
| 4 | Price | `0x08` and 8 bytes, little-endian |
| 5 | Window | `0x08` and 8 bytes, little-endian |
| 6 | Venue scheme | `0x01` and 1 byte |
| 7 | Venue key | `0x20` and 32 bytes |
| 8 | Fee | `0x08` and 8 bytes, little-endian |
| 9 | Registry covenant id | `0x20` and 32 bytes |
| 10 | Deed template hash | `0x20` and 32 bytes |
| 11 | Deed prefix length | `0x08` and 8 bytes, little-endian |
| 12 | Deed suffix length | `0x08` and 8 bytes, little-endian |
| 13 | Salt | `0x24` and 36 bytes |

The name is the bare lowercase name, without `.k`. The salt is the outpoint of the deed that the
listing transaction spends: the 32 bytes of its transaction id, as its hex spells them, and then
its index as 4 bytes, little-endian. The index of a deed is always 0. Because of the salt, no two
listings share an escrow.

On mainnet, fields 9 to 12 are these:

| Field | Value |
|---|---|
| Registry covenant id | `ee2128c03dfac7f6d74734bb3c879bd999434c47a55945b8a6daae2a1e4a21de` |
| Deed template hash | `238a8d0a5af1ea136d50dd49973487929b8e713a84f1bd1650e97776b9a3f5a0` |
| Deed prefix length | 1 |
| Deed suffix length | 3116 |

To make sure that your encoder is correct, compile the source with silverscript and these 13
values as constructor arguments. The bytecode must be the same as your script. The test
`a_fixed_listing_keeps_its_escrow` in `tests/escrow.rs` gives one listing with its owner hash
and its addresses.

The escrow's owner hash is `blake2b-256(script)`. Its script public key is
`OP_BLAKE2B <hash> OP_EQUAL` (`aa 20 <hash> 87`), and its address is the P2SH address of that
hash.

## Open the listing

The listing transaction has this shape. Funding inputs and change come after the outputs below.

| Seat | Content |
|---|---|
| Input 0 | The seller's ACTIVE deed, through the registry's `transfer` with owner type `0x03` and the escrow's owner hash as the new owner. The seller signs it with `SIGHASH_ALL`. Compute budget 100. |
| Output 0 | The deed continuation of exactly BOND, which is 1 KAS on mainnet. It goes to the deed address of the new state. It is bound to the registry covenant id, with input 0 as its authorizing input. |
| Output 1 | The reserve: exactly 0.5 KAS (`50000000`) at the escrow's address. |
| Payload | The listing payload below. |

The reserve must be output 1 and must hold exactly 0.5 KAS. `expire` accepts only that output. If
the reserve is somewhere else, a listing with a window never expires.

The transaction must not carry a card. A buyer saves records afterwards through the registry.

## The listing payload

The dotk market finds a listing through this payload. It is 58 bytes:

| Bytes | Field |
|---|---|
| 0 to 3 | The magic `dotm` (`64 6f 74 6d`) |
| 4 | The version, `0x02` |
| 5 to 12 | Price, 8 bytes, little-endian |
| 13 to 20 | Fee, 8 bytes, little-endian |
| 21 to 24 | Window, 4 bytes, little-endian |
| 25 | Venue scheme |
| 26 to 57 | Venue key |

The payload must state the same terms as the escrow state. A payload that differs rebuilds to
another escrow, and the dotk market refuses the listing.

## Close the listing

Every close seats the deed at input 0 and an escrow UTXO at input 1. The deed input spends
through the registry's `transfer` with no signature and with witness index 1. The index points at
the escrow, which is the deed's owner. The compute budget of the deed input is 8.

Each escrow signature script is the entrypoint's ABI encoding, and then a push of the escrow
script. The deed prefix and the deed suffix arguments are the bytes of the registry's deed
template around its state.

### Buy

| Seat | Content |
|---|---|
| Input 1 | Any UTXO at the escrow's address, usually the listing's reserve. The script sets no value for it. Entry `buy(buyerSig, buyerType, buyer, deedPrefix, deedSuffix)`. The buyer signs with `SIGHASH_ALL` or `SIGHASH_ALL\|ANYONECANPAY`. Compute budget 64. |
| Output 0 | The deed continuation to the buyer, BOND, bound to the registry covenant id. |
| Output 1 | At least the price less the fee, plus 0.5 KAS, to the seller's P2PK script. |
| Output 2 | At least the fee, to your P2PK script. |

A P2PK script is `20 <key> ac` for a schnorr key. For an ECDSA key, it is
`21 <02 or 03> <x> ab`, with `02` for an even y and `03` for an odd y.

### Cancel

The seller signs entry `cancel(sellerSig, deedPrefix, deedSuffix)` with `SIGHASH_ALL` or
`SIGHASH_ALL|ANYONECANPAY`. Output 0 sends the deed back to the seller. Output 1 must not be a
P2SH output. The compute budget of input 1 is 64.

### Expire

Anyone runs entry `expire(deedPrefix, deedSuffix)`, with no signature, after the window. Input 1
must be the listing's own reserve, output 1 of the listing transaction. Its sequence is the
window, and its compute budget is 24. Output 0 sends the deed back to the seller. Output 1 must not
be a P2SH output. A listing with a zero window never expires.

The dotk market runs `expire` for every listing whose window passed, also for yours, and it keeps
the reserve as its bounty.

## Make sure that the dotk market shows your listing

The dotk market API is at `https://market-api.dotk.name/v1` on mainnet and at
`https://market-api-tn10.dotk.name/v1` on testnet-10. Its reference is at the same address.

1. Wait about 5 seconds after the listing transaction is accepted.
2. Read `GET /v1/listings/<name>`. The answer shows your venue and your fee.
3. If the answer is 404, send the terms with `POST /v1/listings`. The answer says why a listing
   does not prove.

The body of the post is this:

```json
{
  "name": "<name>",
  "payload": "<the listing payload, hex>",
  "sellerType": 0,
  "seller": "<the seller key, 64 hex characters>",
  "saltTxid": "<the deed outpoint's transaction id>",
  "saltIndex": 0
}
```

To prove a listing yourself, run `./verify.sh --name <name>`, with `--network testnet-10` on
testnet-10. The [README](README.md) explains the checks.

Run your first listings on testnet-10 before you open one on mainnet.
