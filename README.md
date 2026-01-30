# Perpetual Futures Trading Engine

A Rust implementation of the core math and state management for a perpetual futures exchange.

## What is this?

Imagine you want to bet on whether Bitcoin's price will go up or down, but you don't want to actually buy Bitcoin. That's what perpetual futures let you do.

This project is the "brain" of a trading platform that handles these bets. It's not a full exchange (no networking, no database, no UI), just the core logic that tracks who owns what, calculates profits and losses, and decides when someone's gone broke.

## Key Concepts

### The Basic Idea

You deposit $1,000 as collateral. With 10x leverage, you can control a $10,000 position. If Bitcoin goes up 5%, you make $500 (5% of $10,000). If it goes down 5%, you lose $500. Your $1,000 collateral is your "skin in the game" that covers potential losses.

### Long vs Short

**Long**: You're betting the price goes up. You profit when it rises, lose when it falls.

**Short**: You're betting the price goes down. You profit when it falls, lose when it rises. 

### Why "Perpetual"?

Traditional futures have an expiration date. Perpetuals don't expire, they run forever. To keep the perpetual price aligned with the real market price, there's a "funding rate" where one side pays the other every 8 hours.

### Margin and Leverage

**Collateral/Margin**: The money you put up as a safety deposit.

**Leverage**: How much you multiply your buying power. 10x means $1,000 controls $10,000.

**Initial Margin**: What you need to open a position.

**Maintenance Margin**: The minimum you need to keep it open. Fall below this and you get liquidated.

### Liquidation

If your losses eat into your collateral too much, the system forcibly closes your position before you owe more than you deposited. This protects the platform and other traders from bad debt.

Think of it like a margin call in stock trading, but automatic and instant.

## What This Code Does

### Core Modules

**types.rs**: Basic building blocks. Price, Quote (dollar amounts), SignedSize (positive for long, negative for short), Leverage, etc.

**margin.rs**: Calculates how much collateral you need. Bigger positions require proportionally more margin (dynamic leverage tiers).

**funding.rs**: The mechanism that keeps perpetual prices anchored to reality. Calculates who pays whom and how much.

**mark_price.rs**: Determines the "fair" price used for calculations. Blends the exchange's order book price with external oracle prices to resist manipulation.

**position.rs**: Tracks open positions. Calculates unrealized PnL (paper gains/losses) and handles position increases/decreases.

**liquidation.rs**: Decides when a position is underwater and needs to be forcibly closed. Calculates penalties and handles the insurance fund.

**account.rs**: Manages user accounts. Tracks balances, deposits, withdrawals, and all open positions.

**events.rs**: Every state change emits an event. Useful for auditing, replaying history, and notifying external systems.

### Data Flow

```
User deposits collateral
        ↓
Opens a position (long or short)
        ↓
Price moves → PnL changes
        ↓
Every 8 hours → Funding payments settle
        ↓
If equity < maintenance margin → Liquidation
        ↓
User closes position → Collateral + PnL returned
```

## Architecture Decisions

**Why Rust?** Memory safety, no garbage collection pauses, and the type system catches bugs at compile time.

**Why rust_decimal?** Floating point math has rounding errors. Financial systems need exact decimal arithmetic. This library gives us 28 digits of precision.

**Why isolated margin?** Each position has its own collateral bucket. If one position blows up, it doesn't drag down your other positions. Simpler to reason about than cross-margin (where everything shares one pool).

**Why events?** Makes the system auditable and deterministic. You can replay all events to reconstruct any historical state.

## Running Tests

```bash
cargo test
```

97 tests covering unit tests for each module plus property-based stress tests.

## Running Simulation

```bash
cargo run
```

Runs 7 simulation scenarios demonstrating order matching, position lifecycle, PnL tracking, funding settlement, and liquidation cascades.

## Project Status

The core engine is complete with order book matching, position management, funding settlement, and liquidation execution.

## File Structure

```
src/
├── lib.rs          # Module exports
├── main.rs         # Simulation scenarios
├── types.rs        # Core primitives
├── margin.rs       # Margin requirements
├── funding.rs      # Funding rate logic
├── mark_price.rs   # Price derivation
├── position.rs     # Position tracking
├── liquidation.rs  # Liquidation logic
├── account.rs      # Account management
├── events.rs       # State change events
├── order.rs        # Order types and order book
├── market.rs       # Market configuration and state
└── engine/         # Trading engine
    ├── mod.rs      # Module exports
    ├── config.rs   # Engine configuration
    ├── core.rs     # Core engine struct
    ├── orders.rs   # Order execution
    ├── positions.rs # Position management
    ├── pricing.rs  # Mark price updates
    ├── funding.rs  # Funding settlement
    ├── liquidations.rs # Liquidation checks
    └── results.rs  # Result types and errors

tests/
└── property_tests.rs  # Randomized stress tests

docs/
└── SPEC.md         # Technical specification
```

## Want to Learn More?

The [SPEC.md](docs/SPEC.md) file has all the formulas and parameters. But honestly, reading the code with the tests is probably more useful. Start with `types.rs`, then `position.rs`, then follow your curiosity.
