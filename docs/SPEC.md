# Technical Specification v0.1

This document covers the math and rules. For a gentler introduction, see the README.

## How Margin Works

When you open a position, you need collateral. The amount depends on your leverage.

**Initial Margin** is what you need upfront:

```
initial_margin = position_size × price × (1 / leverage)
```

With 10x leverage on a $50,000 BTC position, you need $5,000.

**Maintenance Margin** is the minimum to keep your position alive. We set it at 50% of initial margin. So that $5,000 initial margin means $2,500 maintenance margin. Drop below $2,500 in equity and you get liquidated.

### Position Size Limits

Bigger positions get lower max leverage. This prevents whales from taking on too much risk.

| Position Size | Max Leverage | Required Margin |
|--------------|--------------|-----------------|
| Under $100k  | 50x          | 2%              |
| $100k-$500k  | 20x          | 5%              |
| $500k-$2M    | 10x          | 10%             |
| $2M-$10M     | 5x           | 20%             |
| Over $10M    | 5x           | 20%             |

## Mark Price

We don't use the raw order book price for PnL and liquidations. That's too easy to manipulate.

Instead, we blend the order book mid-price with an external oracle price (aggregated from multiple exchanges). The formula:

```
mark_price = index_price × (1 + smoothed_premium)
```

The premium is how far the local price deviates from the oracle. We clamp it to ±5% and smooth it with an exponential moving average. This prevents sudden spikes from triggering unfair liquidations.

## Funding Rate

Perpetuals have no expiration, so we need another mechanism to keep prices aligned with spot markets. That's funding.

Every 8 hours, one side pays the other:

- If perpetual trades at a premium (above spot), longs pay shorts.
- If perpetual trades at a discount (below spot), shorts pay longs.

The payment formula:

```
funding_payment = position_size × mark_price × funding_rate
```

The rate is capped at ±1% per 8-hour period. There's also a tiny base interest rate (0.01%) that slightly favors shorts, matching how traditional futures work.

Funding is zero-sum. Every dollar paid by longs goes to shorts, and vice versa.

## Liquidation

You get liquidated when:

```
account_equity < maintenance_margin
```

Your equity is collateral plus unrealized PnL minus any pending funding you owe.

**Liquidation price for a long position:**

```
liq_price = entry_price × (1 - (initial_margin_fraction - maintenance_margin_fraction))
```

At 10x leverage (10% IM, 5% MM), your liquidation price is 5% below entry. For shorts, it's 5% above.

When liquidated, you pay a 1% penalty. Half goes to whoever executed the liquidation (incentive), half goes to an insurance fund.

If your equity goes negative (bad debt), the insurance fund covers it. If the insurance fund runs dry, profitable positions get auto-deleveraged (ADL) to cover losses. This is the nuclear option and rarely happens.

## Position Math

**Unrealized PnL:**

```
pnl = size × (current_price - entry_price)
```

For longs (positive size), you profit when price rises. For shorts (negative size), the negative times negative gives positive PnL when price falls.

**Adding to a position:**

Your new entry price is the weighted average:

```
new_entry = (old_size × old_entry + new_size × fill_price) / total_size
```

**Reducing a position:**

Entry price stays the same. The PnL on the closed portion gets realized immediately.

## State Machine

Every action follows a clear flow.

**Opening a position:**
1. Check you have enough free margin.
2. Lock collateral from your balance.
3. Create position record with entry price and funding index.
4. Emit event.

**Price update:**
1. Oracle pushes new index price.
2. Calculate new mark price.
3. Recalculate all positions' PnL and margin status.
4. Flag any positions for liquidation.
5. Emit event.

**Funding settlement:**
1. Calculate time since last funding.
2. Pro-rate the 8-hour funding for elapsed time.
3. Update cumulative funding index.
4. Debit/credit accounts.
5. Emit events.

**Liquidation:**
1. Verify position is actually liquidatable.
2. Close at mark price.
3. Deduct penalty.
4. Pay liquidator and insurance fund.
5. Handle any bad debt.
6. Emit event.

## Invariants

Things that must always be true:

1. Nobody's equity drops below maintenance margin without triggering liquidation.
2. Funding payments net to zero across all positions.
3. No account has negative balance.
4. Same inputs always produce same outputs (deterministic).
5. Every state change has a corresponding event.

## Default Parameters

```
Margin:
  max_leverage: 50x
  maintenance_ratio: 50% of initial margin

Mark Price:
  max_premium: 5%
  ema_smoothing: 0.1
  oracle_weight: 75%

Funding:
  max_rate: 1% per 8 hours
  base_interest: 0.01% per 8 hours
  period: 8 hours

Liquidation:
  penalty: 1%
  liquidator_cut: 50%
  max_single_liquidation: $1M
```

## Implementation Notes

We use `rust_decimal` for all money math. Floating point would introduce rounding errors. This gives us 28 decimal digits of precision.

All sizes are signed. Positive means long, negative means short. This makes the PnL formula work for both directions without branching.

Timestamps are milliseconds since Unix epoch. Simple and unambiguous.

Events are the source of truth. You can rebuild any state by replaying events from the beginning.
