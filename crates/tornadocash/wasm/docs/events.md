# Decode supplied Tornado logs

`decodeEvent` uses the core decoder to read deposit or withdrawal events from
caller-supplied logs.

With the generated module loaded and a caller-supplied `log`:

```js
const event = decodeEvent(log);
if (event?.kind === 'deposit') {
  console.log(event.commitment, event.leafIndex, event.blockNumber);
}
```
