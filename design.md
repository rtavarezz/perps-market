# How This Trading Engine Works

A guide for anyone new to perpetual futures and how this system is designed.

## What Are Perpetual Futures

Imagine you want to bet on Bitcoin going up or down without actually buying Bitcoin. That is what perpetual futures let you do. You are trading a contract that tracks the price of Bitcoin, not Bitcoin itself.

The word perpetual means it never expires. Traditional futures have an end date where you have to settle up. Perpetuals run forever. You can hold your position for five minutes or five years.

## The Basic Trade

You deposit money as collateral, say 1000 dollars. With 10x leverage you can control a 10000 dollar position. If Bitcoin is at 50000 and you go long on 0.2 BTC, you are betting on 10000 dollars worth of Bitcoin.

If the price goes up 5 percent to 52500, your position is now worth 10500. You made 500 dollars profit on your 1000 dollar deposit. That is a 50 percent return because of the 10x leverage.

If the price goes down 5 percent to 47500, your position is worth 9500. You lost 500 dollars. Your collateral is now only 500 dollars.

## Long vs Short

Going long means you profit when the price goes up. Going short means you profit when the price goes down.

Long position with 1 BTC at 50000 entry. Price goes to 52000. Your profit is 1 times (52000 minus 50000) which equals 2000 dollars.

Short position with 1 BTC at 50000 entry. Price goes to 48000. Your profit is 1 times (50000 minus 48000) which equals 2000 dollars. You made money because the price dropped.

## System Architecture

```
User Action (deposit, place order, etc)
         │
         ▼
    ┌─────────┐
    │ Engine  │  ← Orchestrates everything
    └────┬────┘
         │
    ┌────┴────┬────────┬─────────┬──────────┐
    ▼         ▼        ▼         ▼          ▼
 Account   Order    Position  Funding  Liquidation
 (balance) (book)   (pnl)     (rates)  (margin check)
    │         │        │         │          │
    └────┬────┴────────┴─────────┴──────────┘
         ▼
      Types (Price, Size, Leverage, etc)
```

## Core Components

### Types Layer

Everything starts with the basic building blocks. Price represents a dollar value per unit. Size represents how much you are trading, positive for long and negative for short. Leverage is how much you multiply your buying power. These are separate types so the compiler catches mistakes like passing a price where a size is expected.

### Account Module

Each trader has an account. The account tracks your balance, which is how much collateral you deposited. It also tracks all your open positions and your realized profit and loss from closed trades.

When you deposit, your balance goes up. When you withdraw, it goes down. When you open a position, collateral gets reserved. When you close a position, your profit or loss gets added to your balance.

### Order Book

The order book is where buyers and sellers meet. Buy orders are called bids. Sell orders are called asks. The highest bid and lowest ask form the spread.

When you place a limit order, it sits on the book waiting for someone to match it. When you place a market order, it immediately executes against the best available price on the other side.

Orders follow price time priority. Better prices get matched first. At the same price, earlier orders get matched first.

### Position Module

A position represents your open trade. It stores the size, the entry price, your collateral, and when you opened it.

The position calculates your unrealized profit and loss. This is your paper gain or loss based on current price minus your entry price. It becomes realized when you close the position.

You can increase a position by adding to it. The entry price becomes a weighted average. You can reduce a position by closing part of it. You can flip from long to short in a single trade.

### Market State

Each market tracks its order book, current price, funding state, and open interest. Open interest is the total size of all open positions, tracked separately for longs and shorts.

## How Margin Works

Margin is the collateral backing your position. There are two levels.

Initial margin is what you need to open a position. With 10x leverage, initial margin is 10 percent of the position value. For a 10000 dollar position, you need 1000 dollars.

Maintenance margin is the minimum to keep your position open, usually half of initial margin. For that same position, maintenance is 500 dollars. If your equity falls below this, you get liquidated.

Equity is your collateral plus unrealized profit minus unrealized loss. If price moves against you and equity drops below maintenance margin, the system closes your position.

### Leverage Tiers

Big positions get less leverage. A 50000 dollar position can use 50x. A 500000 dollar position is capped at 20x. A 2 million dollar position is capped at 10x. This protects the protocol from whale sized blowups.

## How Funding Works

Funding is the mechanism that keeps the perpetual price close to the real spot price. Every 8 hours, one side pays the other.

If the perpetual is trading above spot, longs pay shorts. This discourages going long and encourages going short, pushing the price back down.

If the perpetual is trading below spot, shorts pay longs. This encourages going long and discourages going short, pushing the price back up.

The payment is calculated as position size times mark price times funding rate. A 0.01 percent rate on a 10000 dollar position is 1 dollar.

Funding is zero sum. Every dollar paid by longs is received by shorts. The protocol does not take a cut.

## How Liquidation Works

Liquidation protects the system from bad debt. When your equity falls below maintenance margin, your position gets forcibly closed.

The liquidation price is calculated when you open. For a long at 50000 with 10x leverage and 5 percent maintenance, liquidation triggers around 46250. The exact formula accounts for your entry price, leverage, and maintenance requirement.

When liquidated, you lose your remaining collateral plus a penalty. Part of the penalty goes to the liquidator who executed it, part goes to the insurance fund.

### Bad Debt and Insurance

If your position goes so far underwater that equity is negative, that is bad debt. The insurance fund covers it. If the insurance fund runs dry, auto deleveraging kicks in where profitable traders on the other side get their positions reduced.

### Auto Deleveraging (ADL)

When insurance cannot cover bad debt, the system looks at the other side. If a long position created bad debt, the most profitable shorts get partially closed. Their profit gets used to cover the hole. This is rare but necessary to keep the system solvent.

## Data Flow For a Trade

1. User submits a buy order for 0.5 BTC at 50000.
2. Engine validates the order and checks account has enough margin.
3. Order goes to the order book and matches against existing sell orders.
4. For each match, a fill is created recording the price and size.
5. Position module updates or creates the position with new size and averaged entry.
6. Account module reserves collateral and updates balances.
7. Market state updates open interest.
8. Events are emitted for each state change.

## Order Types Supported

Market orders execute immediately at the best available price. Use these when you want in or out now.

Limit orders sit on the book at your specified price. They only execute if someone matches your price or better.

Stop loss orders trigger when price hits a threshold. If you are long and set a stop at 48000, it converts to a market sell when price drops to 48000.

Take profit orders work the same way but in the profit direction. Long with take profit at 55000 sells when price rises to 55000.

Trailing stops follow the price. If price goes up, the stop moves up. If price reverses, the stop stays put and eventually triggers.

## Risk Controls

### Circuit Breakers

Circuit breakers pause trading when prices move too fast. If price drops 15 percent in one minute, something is probably wrong. Better to halt and investigate than let cascading liquidations drain the insurance fund.

### Position and OI Limits

Position limits cap how much any single account can hold. Open interest limits cap total exposure. These prevent the system from taking on more risk than it can handle.

### Price Staleness

If the oracle price is too old, the system rejects new orders. Stale prices mean the market has moved and trades would happen at wrong prices.

## What is Mocked

The trading engine is complete but runs in memory. Three things are simulated.

Price feeds use fake data instead of real oracles like Pyth or Chainlink. In production you need real market prices.

Settlement uses an in memory map instead of blockchain transactions. In production this would be smart contracts holding real funds.

Custody flows are simulated. Deposits and withdrawals are just balance changes. In production these would be actual token transfers.

## File Structure

```
src/
    types.rs        Core primitives, Price, Size, Leverage
    account.rs      User accounts and balances
    position.rs     Open positions and pnl calculation
    order.rs        Order book and matching
    margin.rs       Margin requirements
    funding.rs      Funding rate logic
    liquidation.rs  Liquidation detection and execution
    risk.rs         Circuit breakers and limits
    market.rs       Market state
    events.rs       Event log for auditing
    adl.rs          Auto deleveraging
    conditional.rs  Stop loss, take profit, trailing stop
    engine/         Main engine that ties it all together
```

## Testing

184 tests verify the math is correct. Unit tests check each module in isolation. Property tests throw random inputs at the system and verify invariants hold.

Key invariants tested include pnl is zero when price equals entry, funding payments sum to zero across all accounts, open interest for longs equals open interest for shorts, and liquidation triggers at the calculated price.

Run tests with cargo test. Run the simulation with cargo run to see 16 scenarios demonstrating order matching, position lifecycle, funding settlement, and liquidation cascades.

## Key Formulas

Unrealized PnL equals size times (mark price minus entry price). Positive size for long, negative for short.

Notional value equals absolute size times price. This is the dollar value of your position.

Initial margin equals notional divided by leverage. 10x leverage means 10 percent margin.

Maintenance margin equals initial margin times maintenance ratio. Usually 50 percent of initial.

Liquidation price for long equals entry times (1 minus initial margin fraction plus maintenance fraction).

Funding payment equals size times mark price times funding rate.