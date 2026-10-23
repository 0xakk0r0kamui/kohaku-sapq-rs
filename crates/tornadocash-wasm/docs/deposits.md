# Deposit preparation

`Deposit` wraps the core Tornado deposit. Construct it from an existing `Pool`
and `Note`; both inputs are borrowed and remain usable. The deposit owns its
canonical core values independently.

The bindings expose the core's preparation methods:

| Method | JavaScript result |
| --- | --- |
| `depositData()` | A plain `Call` for depositing the note into the pool. |
| `approvalData()` | A plain ERC20 approval `Call`, or `undefined` for native pools. |

Both methods use the same TypeScript shape:

```ts
interface Call {
  to: Hex;
  data: Hex;
  value: bigint;
}
```

The core supplies all three fields. For a deposit, `to` is the pool address and
`data` encodes `deposit(commitment)`. `value` is the pool's native denomination,
or `0n` for an ERC20 pool. For an ERC20 approval, `to` is the token address,
`data` encodes `approve(pool, amount)`, and `value` is explicitly `0n`.
Addresses use checksum formatting and calldata uses lowercase hex.

These synchronous methods prepare unsigned call data locally. The caller handles
existing allowance, transaction fees, signing, submission and confirmation.
An executed ERC20 approval grants spending permission; the deposit is a separate
transaction. Serialization errors throw JavaScript `Error`s.

Call `free()` on owned `Deposit`, `Pool` and `Note` wrappers when finished.
Returned calls are ordinary JavaScript objects managed by JavaScript's garbage
collector.

## Usage

Given an existing `Pool` and `Note`, read the prepared calls and pass them to your
transaction tooling:

```js
const deposit = new Deposit(pool, note);
const call = deposit.depositData();
const approval = deposit.approvalData(); // undefined for a native pool

deposit.free();
```
