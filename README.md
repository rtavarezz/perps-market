# Perpetual Futures Trading Engine

Rust implementation of the core logic for a perpetual futures exchange.

## What This Is

This is the trading engine, the part that handles order matching, margin calculations, funding rates, liquidations, and position tracking. It runs in memory as a simulation.

## What Works

Order book with limit and market orders, partial fills, price time priority.

Position management including opening, closing, increasing, reducing, and flipping from long to short.

Margin system with initial margin, maintenance margin, and tiered leverage up to 50x.

Funding rate settlement every 8 hours with premium index calculation.

Liquidation detection, penalty calculation, insurance fund, and auto deleveraging.

Risk controls including circuit breakers, position size limits, and price deviation checks.

Conditional orders including stop loss, take profit, and trailing stops.

## What Is Mocked

Price feeds use mock data, not real oracles.

Settlement uses in memory storage, not blockchain transactions.

Custody flows are simulated, no real deposits or withdrawals.

## Tests

184 tests covering margin math, funding calculations, liquidation scenarios, order matching, and stress tests.

```
cargo test
```

## Simulation

16 scenarios demonstrating order matching, position lifecycle, funding settlement, liquidations, and risk controls.

```
cargo run
```

## Files

types.rs has core primitives like Price, Size, Leverage.

margin.rs calculates margin requirements.

funding.rs handles funding rate logic.

position.rs tracks positions and calculates pnl.

liquidation.rs handles liquidation logic and insurance fund.

order.rs has order types and the order book.

account.rs manages user accounts and balances.

engine folder contains the main trading engine that ties everything together.

api.rs, config.rs, price_feed.rs, liquidity.rs, custody.rs, and settlement.rs are integration layer modules with mock implementations.
