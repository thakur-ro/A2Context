# Follow a held exchange

The customer has approved two exchanges, then puts the second one on hold. The workflow still has a cached approval. All records, proposals and order IDs in this walkthrough are synthetic. A deterministic fixture produces the proposal and a disposable SQLite sandbox applies the approval check.

```sh
cargo run --locked -- demo
```

Open the printed address and select **Exchange agent**. Ask **What should we test before the next exchange?** Then ask **What did the customer ask for, and did the approval check change the outcome?** The comparison shows the second order with and without the check. The first order keeps its earlier state.

[![Walkthrough of the synthetic exchange](media/exchange-approval.gif)](media/exchange-approval.mp4)

[Connect a real workflow](getting-started.md)
