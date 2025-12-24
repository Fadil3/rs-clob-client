This is a dangerous strategy if implemented naively. To answer your question directly: **No, you do not always profit.**

While the math () is "risk-free," the **execution** is not.

### The Risks (Why you might lose that $20)

1. **Legging Risk (The #1 Killer):**

- **Scenario:** You find an arbitrage opportunity in the "2024 Election" market. You need to buy `YES` on Trump, Harris, and "Other".
- **The Bug:** You send 3 buy orders simultaneously.
- **The Result:** Your order for Trump fills. Your order for Harris fills. But your order for "Other" **fails** because someone else bought it 5ms before you, or the price moved up.
- **Outcome:** You are now holding a naked gambling position on Trump + Harris. You are no longer arbitrage trading; you are betting. If "Other" wins, you lose everything.

2. **Liquidity Gaps:**

- Often, the arbitrage exists because one outcome is illiquid. You might see a price of $0.05 for a long-shot candidate, but there are only $2 worth of shares available. If you try to buy $20 worth, you will eat through the order book, raising the average price significantly and destroying your profit margin.

3. **Execution Latency:**

- You are competing with bots hosted in the same data center as the matching engine. If your Rust bot is running on a laptop in Jakarta, you will lose the race for the obvious arbs.

---

### Product Requirements Document (PRD)

**Project Name:** `polybot-rs` (NegRisk Arbitrage Engine)
**Language:** Rust
**Target Platform:** Polygon (Polymarket CLOB)
**Objective:** Detect and execute "Negative Risk" arbitrage opportunities where the sum of `BestAsk` across all mutually exclusive outcomes is < $1.00 (minus fees).

#### 1. Functional Requirements

**A. Market Scanning (The "Eyes")**

- **Input:** Fetch all active markets with `active: true` and `closed: false`.
- **Filter:** Must only process markets with `negrisk: true` (Negative Risk capable).
- **Grouping:** Group individual `token_id`s by their parent `condition_id`. (e.g., Group "Trump", "Harris", "Newsom" together).

**B. Opportunity Detection (The "Brain")**

- **Logic:**

- **Threshold:** Configurable `MIN_PROFIT_BPS` (e.g., 50 bps or 0.5%). If profit is less, ignore (gas/risk is not worth it).
- **Liquidity Check:** Ensure `min(Size_{ask, i})` across all outcomes > `MIN_TRADE_SIZE`.

**C. Execution Engine (The "Hands")**

- **Concurrency:** Must submit `CreateOrder` payloads for ALL outcomes in the group simultaneously (using `tokio::spawn` or `join_all`).
- **Fail-Safe:** If any single order in the batch fails (Rejected/fillOrKill), the bot must immediately attempt to **dump** the filled positions to exit the risk, even at a small loss.

#### 2. Non-Functional Requirements

- **Latency:** Critical path (Tick → Calc → Order) must be < 50ms.
- **Safety:** Hardcoded `MAX_SPEND` per trade ($5.00) to prevent draining wallet on a bug.
- **Logging:** Structured JSON logs for every decision (Win/Loss/Miss).

#### 3. Architecture Diagram

**Text Description of Flow:**

1. **WebSocket Feed:** Subscribes to `orderbook` channel for target markets (faster than REST polling).
2. **ArbCalculator:** In-memory structure updates prices in microseconds.
3. **Trigger:** When `Sum < 0.99`, sends signal to `OrderManager`.
4. **OrderManager:** Signs transactions locally (Rust `ethers`) and blasts them to `Relayer` API.

#### 4. Rust Data Structures (Core)

```rust
// The "State" of a single event (e.g. Election)
struct MarketGroup {
    condition_id: String,
    outcomes: Vec<Outcome>, // [Trump, Harris, Other]
}

struct Outcome {
    token_id: String,
    best_ask: f64,
    liquidity: f64,
}

// Result of the calculation pass
enum Decision {
    Execute {
        profit_margin: f64,
        orders: Vec<OrderArgs>,
    },
    Ignore,
}

```

#### 5. MVP Implementation Plan (Phase 1)

- **Goal:** Do not lose money.
- **Strategy:** "Paper Trading" Mode.
- **Action:** Run the bot live, connect to real websockets, process real prices, but **log** the trade instead of sending it.
- **Success Metric:** Let it run for 24 hours. Check logs. Did it find arbs? If it had executed, would it have failed due to liquidity changes (compare timestamp of your log vs. next tick on chart)?

### Next Step for You

Since you want to start, I can generate the **"Market Grouping" logic** in Rust. This is the hardest part: taking a flat list of 1,000 markets from the API and correctly grouping them into "Events" so you can sum their probabilities.

Shall I write that `struct` and grouping logic for you?
