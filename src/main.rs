//! Perpetual DEX Core Simulation.
//!
//! Demonstrates the full trading engine lifecycle including order matching,
//! position tracking, funding settlement, and liquidation cascades.
//!
//! Adds demonstrations for risk management, circuit breakers,
//! auto deleveraging, and conditional orders. also adds integration layer demos: price feeds, liquidity pools,
//! custody flows, and settlement batching.

use perps_core::*;
use perps_core::api::{EngineCommand, validate_command};
use perps_core::config::{IntegrationConfig, Environment};
use perps_core::custody::{CustodyManager, CustodyConfig, DepositRequest, CollateralType};
use perps_core::liquidity::{SharedPool, PoolConfig};
use perps_core::price_feed::{PriceAggregator, PriceFeedConfig, PriceUpdate, TwapCalculator};
use perps_core::settlement::{SettlementManager, SettlementInstruction, TransferReason, InMemorySettlement, SettlementBackend};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

fn main() {
    println!("Perpetual DEX Core Engine Simulation");
    println!("Single Market, Isolated Margin, Full Lifecycle\n");

    scenario_1_basic_trading();
    scenario_2_multiple_traders();
    scenario_3_position_lifecycle();
    scenario_4_price_movement_and_pnl();
    scenario_5_funding_settlement();
    scenario_6_liquidation_cascade();
    scenario_7_stress_test();

    println!("Hardening Scenarios ===\n");

    scenario_8_circuit_breakers();
    scenario_9_conditional_orders();
    scenario_10_adl_ranking();
    scenario_11_near_margin_edge();

    println!("Integration Scenarios ===\n");

    scenario_12_price_aggregation();
    scenario_13_liquidity_pool();
    scenario_14_custody_flows();
    scenario_15_settlement_batching();
    scenario_16_config_presets();

    println!("\nAll simulations completed successfully.");
}

/// Basic order matching between two traders.
fn scenario_1_basic_trading() {
    println!("Scenario 1: Basic Order Matching\n");

    let mut engine = Engine::new(EngineConfig::default());
    engine.add_market(MarketConfig::btc_perp());

    let alice = engine.create_account();
    let bob = engine.create_account();

    engine.deposit(alice, Quote::new(dec!(50000))).unwrap();
    engine.deposit(bob, Quote::new(dec!(50000))).unwrap();
    engine.update_index_price(MarketId(1), Price::new_unchecked(dec!(50000))).unwrap();

    println!("  Alice and Bob each deposit $50,000");
    println!("  Oracle price set to $50,000\n");

    let sell_result = engine
        .place_limit_order(bob, MarketId(1), Side::Short, dec!(1.0), Price::new_unchecked(dec!(50000)), TimeInForce::GTC)
        .unwrap();

    println!("  Bob places SELL 1 BTC @ $50,000, posted: {}", sell_result.is_posted);

    let buy_result = engine
        .place_market_order(alice, MarketId(1), Side::Long, dec!(0.5))
        .unwrap();

    println!("  Alice places BUY 0.5 BTC market order");
    println!("  Filled: {} BTC @ ${}\n", buy_result.filled_size, buy_result.average_price.unwrap());

    let alice_pos = engine.get_account(alice).unwrap().get_position(MarketId(1)).unwrap();
    let bob_pos = engine.get_account(bob).unwrap().get_position(MarketId(1)).unwrap();

    println!("  Alice: {} BTC @ ${}", alice_pos.size, alice_pos.entry_price);
    println!("  Bob: {} BTC @ ${}", bob_pos.size, bob_pos.entry_price);

    let market = engine.get_market(MarketId(1)).unwrap();
    println!("  Open interest: {} long, {} short\n", market.open_interest_long, market.open_interest_short);
}

/// Order book depth with multiple market makers.
fn scenario_2_multiple_traders() {
    println!("Scenario 2: Order Book Depth\n");

    let mut engine = Engine::new(EngineConfig::default());
    engine.add_market(MarketConfig::btc_perp());

    let mm1 = engine.create_account();
    let mm2 = engine.create_account();
    let taker = engine.create_account();

    for acc in [mm1, mm2, taker] {
        engine.deposit(acc, Quote::new(dec!(100000))).unwrap();
    }

    engine.update_index_price(MarketId(1), Price::new_unchecked(dec!(50000))).unwrap();

    engine.place_limit_order(mm1, MarketId(1), Side::Long, dec!(2.0), Price::new_unchecked(dec!(49900)), TimeInForce::GTC).unwrap();
    engine.place_limit_order(mm1, MarketId(1), Side::Short, dec!(2.0), Price::new_unchecked(dec!(50100)), TimeInForce::GTC).unwrap();
    engine.place_limit_order(mm2, MarketId(1), Side::Long, dec!(1.0), Price::new_unchecked(dec!(49950)), TimeInForce::GTC).unwrap();
    engine.place_limit_order(mm2, MarketId(1), Side::Short, dec!(1.0), Price::new_unchecked(dec!(50050)), TimeInForce::GTC).unwrap();

    let market = engine.get_market(MarketId(1)).unwrap();

    println!("  Best bid: ${}, best ask: ${}", market.order_book.best_bid().unwrap(), market.order_book.best_ask().unwrap());
    println!("  Spread: ${}", market.order_book.spread().unwrap_or(Decimal::ZERO));

    println!("\n  Taker sweeps 2.5 BTC...");
    let result = engine.place_market_order(taker, MarketId(1), Side::Long, dec!(2.5)).unwrap();

    println!("  Filled {} BTC across {} fills\n", result.filled_size, result.fills.len());
}

/// Position lifecycle from open to close.
fn scenario_3_position_lifecycle() {
    println!("Scenario 3: Position Lifecycle\n");

    let mut engine = Engine::new(EngineConfig::default());
    engine.add_market(MarketConfig::btc_perp());

    let trader = engine.create_account();
    let counterparty = engine.create_account();

    engine.deposit(trader, Quote::new(dec!(50000))).unwrap();
    engine.deposit(counterparty, Quote::new(dec!(100000))).unwrap();
    engine.update_index_price(MarketId(1), Price::new_unchecked(dec!(50000))).unwrap();

    engine.place_limit_order(counterparty, MarketId(1), Side::Short, dec!(10.0), Price::new_unchecked(dec!(50000)), TimeInForce::GTC).unwrap();

    println!("  Opening 0.5 BTC long...");
    engine.place_market_order(trader, MarketId(1), Side::Long, dec!(0.5)).unwrap();
    let pos = engine.get_account(trader).unwrap().get_position(MarketId(1)).unwrap();
    println!("  Position: {} BTC @ ${}", pos.size, pos.entry_price);

    println!("  Adding 0.3 BTC...");
    engine.place_market_order(trader, MarketId(1), Side::Long, dec!(0.3)).unwrap();
    let pos = engine.get_account(trader).unwrap().get_position(MarketId(1)).unwrap();
    println!("  Position: {} BTC @ ${}", pos.size, pos.entry_price);

    println!("  Closing 0.2 BTC...");
    engine.place_market_order(trader, MarketId(1), Side::Short, dec!(0.2)).unwrap();
    let pos = engine.get_account(trader).unwrap().get_position(MarketId(1)).unwrap();
    println!("  Position: {} BTC", pos.size);

    println!("  Closing remaining...");
    engine.place_market_order(trader, MarketId(1), Side::Short, dec!(0.6)).unwrap();
    let account = engine.get_account(trader).unwrap();
    println!("  Position closed, balance: ${}, realized PnL: ${}\n", account.balance, account.realized_pnl);
}

/// Price movements and unrealized PnL tracking.
fn scenario_4_price_movement_and_pnl() {
    println!("Scenario 4: Price Movement and PnL\n");

    let mut engine = Engine::new(EngineConfig::default());
    engine.add_market(MarketConfig::btc_perp());

    let long_trader = engine.create_account();
    let short_trader = engine.create_account();

    engine.deposit(long_trader, Quote::new(dec!(10000))).unwrap();
    engine.deposit(short_trader, Quote::new(dec!(10000))).unwrap();
    engine.update_index_price(MarketId(1), Price::new_unchecked(dec!(50000))).unwrap();

    engine.place_limit_order(short_trader, MarketId(1), Side::Short, dec!(1.0), Price::new_unchecked(dec!(50000)), TimeInForce::GTC).unwrap();
    engine.place_market_order(long_trader, MarketId(1), Side::Long, dec!(1.0)).unwrap();

    println!("  Entry @ $50,000");
    print_pnl(&engine, long_trader, short_trader);

    engine.update_index_price(MarketId(1), Price::new_unchecked(dec!(52000))).unwrap();
    println!("  Price rises to $52,000");
    print_pnl(&engine, long_trader, short_trader);

    engine.update_index_price(MarketId(1), Price::new_unchecked(dec!(48000))).unwrap();
    println!("  Price drops to $48,000");
    print_pnl(&engine, long_trader, short_trader);

    println!();
}

fn print_pnl(engine: &Engine, long_id: AccountId, short_id: AccountId) {
    let market = engine.get_market(MarketId(1)).unwrap();
    let mark = market.effective_mark_price().unwrap();

    let long_pos = engine.get_account(long_id).unwrap().get_position(MarketId(1)).unwrap();
    let short_pos = engine.get_account(short_id).unwrap().get_position(MarketId(1)).unwrap();

    let long_pnl = long_pos.unrealized_pnl(mark);
    let short_pnl = short_pos.unrealized_pnl(mark);

    println!("    Long PnL: ${}, Short PnL: ${}", long_pnl, short_pnl);
}

/// Funding rate settlement between longs and shorts.
fn scenario_5_funding_settlement() {
    println!("Scenario 5: Funding Settlement\n");

    let mut engine = Engine::new(EngineConfig::default());
    engine.add_market(MarketConfig::btc_perp());

    let long_trader = engine.create_account();
    let short_trader = engine.create_account();

    engine.deposit(long_trader, Quote::new(dec!(10000))).unwrap();
    engine.deposit(short_trader, Quote::new(dec!(10000))).unwrap();
    engine.update_index_price(MarketId(1), Price::new_unchecked(dec!(50000))).unwrap();

    engine.place_limit_order(short_trader, MarketId(1), Side::Short, dec!(1.0), Price::new_unchecked(dec!(50000)), TimeInForce::GTC).unwrap();
    engine.place_market_order(long_trader, MarketId(1), Side::Long, dec!(1.0)).unwrap();

    let long_before = engine.get_account(long_trader).unwrap().balance;
    let short_before = engine.get_account(short_trader).unwrap().balance;

    println!("  Before funding: long ${}, short ${}", long_before, short_before);

    engine.advance_time(8 * 60 * 60 * 1000);
    let result = engine.settle_funding(MarketId(1)).unwrap();

    let long_after = engine.get_account(long_trader).unwrap().balance;
    let short_after = engine.get_account(short_trader).unwrap().balance;

    println!("  After 8 hours: long ${}, short ${}", long_after, short_after);
    println!("  Funding rate: {:.6}%, {} accounts affected\n", result.funding_rate * dec!(100), result.accounts_affected);
}

/// Liquidation cascade from price crash.
fn scenario_6_liquidation_cascade() {
    println!("Scenario 6: Liquidation Cascade\n");

    let mut engine = Engine::new(EngineConfig::default());
    engine.add_market(MarketConfig::btc_perp());
    engine.fund_insurance(Quote::new(dec!(100000)));

    let conservative = engine.create_account();
    let moderate = engine.create_account();
    let aggressive = engine.create_account();
    let counterparty = engine.create_account();

    engine.deposit(conservative, Quote::new(dec!(20000))).unwrap();
    engine.deposit(moderate, Quote::new(dec!(10000))).unwrap();
    engine.deposit(aggressive, Quote::new(dec!(5000))).unwrap();
    engine.deposit(counterparty, Quote::new(dec!(500000))).unwrap();

    engine.update_index_price(MarketId(1), Price::new_unchecked(dec!(50000))).unwrap();

    engine.place_limit_order(counterparty, MarketId(1), Side::Short, dec!(10.0), Price::new_unchecked(dec!(50000)), TimeInForce::GTC).unwrap();

    engine.place_market_order(conservative, MarketId(1), Side::Long, dec!(0.2)).unwrap();
    engine.place_market_order(moderate, MarketId(1), Side::Long, dec!(0.5)).unwrap();
    engine.place_market_order(aggressive, MarketId(1), Side::Long, dec!(1.0)).unwrap();

    println!("  Positions opened at $50,000");

    for (price, label) in [(dec!(48000), "$48k"), (dec!(45000), "$45k"), (dec!(42000), "$42k"), (dec!(40000), "$40k")] {
        engine.update_index_price(MarketId(1), Price::new_unchecked(price)).unwrap();
        let liqs = engine.check_liquidations(MarketId(1)).unwrap();

        if liqs.is_empty() {
            println!("  {}: no liquidations", label);
        } else {
            for liq in &liqs {
                let name = if liq.account_id == conservative { "conservative" }
                    else if liq.account_id == moderate { "moderate" }
                    else { "aggressive" };
                println!("  {}: {} liquidated, {} BTC, bad debt ${}", label, name, liq.position_size.abs(), liq.bad_debt);
            }
        }
    }

    println!("  Insurance fund: ${}\n", engine.insurance_fund_balance());
}

/// Stress test with many traders and volatile prices.
fn scenario_7_stress_test() {
    println!("Scenario 7: Stress Test\n");

    let mut engine = Engine::new(EngineConfig::default());
    engine.add_market(MarketConfig::btc_perp());
    engine.fund_insurance(Quote::new(dec!(1000000)));

    let num_traders = 20;
    let mut traders = Vec::new();

    for i in 0..num_traders {
        let id = engine.create_account();
        let capital = dec!(5000) + Decimal::from(i) * dec!(2500);
        engine.deposit(id, Quote::new(capital)).unwrap();
        traders.push(id);
    }

    engine.update_index_price(MarketId(1), Price::new_unchecked(dec!(50000))).unwrap();

    println!("  Created {} traders with $5k to $52.5k", num_traders);

    let mut order_count = 0;
    for (i, &trader) in traders.iter().enumerate() {
        let side = if i % 2 == 0 { Side::Long } else { Side::Short };
        let price_offset = Decimal::from((i as i32 - 10) * 50);
        let price = dec!(50000) + price_offset;
        let size = dec!(0.1) + Decimal::from(i % 5) * dec!(0.05);

        if engine.place_limit_order(trader, MarketId(1), side, size, Price::new_unchecked(price), TimeInForce::GTC).is_ok() {
            order_count += 1;
        }
    }

    let market = engine.get_market(MarketId(1)).unwrap();
    println!("  Placed {} orders, {} on book", order_count, market.order_book.order_count());

    let prices = [
        dec!(50500), dec!(51000), dec!(50200), dec!(49000), dec!(48000),
        dec!(49500), dec!(51500), dec!(52000), dec!(50000), dec!(47000),
        dec!(45000), dec!(43000), dec!(44000), dec!(46000), dec!(48000),
    ];

    let mut total_liquidations = 0;
    let mut total_bad_debt = Decimal::ZERO;

    for price in prices {
        engine.update_index_price(MarketId(1), Price::new_unchecked(price)).unwrap();
        let liqs = engine.check_liquidations(MarketId(1)).unwrap();
        total_liquidations += liqs.len();
        for liq in &liqs {
            total_bad_debt += liq.bad_debt.value();
        }
    }

    println!("  Price range: $43k to $52k");
    println!("  Total liquidations: {}", total_liquidations);
    println!("  Total bad debt: ${}", total_bad_debt);

    engine.advance_time(8 * 60 * 60 * 1000);
    if let Ok(funding) = engine.settle_funding(MarketId(1)) {
        println!("  Funding settled, {} accounts", funding.accounts_affected);
    }

    let active = traders.iter().filter(|&&id| engine.get_account(id).unwrap().get_position(MarketId(1)).is_some()).count();
    println!("  Active positions: {}/{}", active, num_traders);
    println!("  Events generated: {}\n", engine.events().len());
}

/// Circuit breaker demonstration.
fn scenario_8_circuit_breakers() {
    println!("Scenario 8: Circuit Breakers\n");

    let params = RiskParams {
        max_price_deviation: dec!(0.10), // 10% max move
        price_window_ms: 60_000,
        ..Default::default()
    };

    let mut risk_state = RiskState::new(MarketId(1));
    let start_time = Timestamp::from_millis(1000000);

    // Normal price update
    let initial_price = Price::new_unchecked(dec!(50000));
    let result = risk_state.record_price(initial_price, start_time, &params);
    println!("  Initial price $50,000: circuit breaker = {}", result.is_some());

    // Small movement, acceptable
    let small_move = Price::new_unchecked(dec!(51000));
    let result = risk_state.record_price(small_move, Timestamp::from_millis(1001000), &params);
    println!("  Move to $51,000 (2%): circuit breaker = {}", result.is_some());

    // Large movement, triggers breaker
    let large_move = Price::new_unchecked(dec!(56000));
    let result = risk_state.record_price(large_move, Timestamp::from_millis(1002000), &params);
    println!("  Move to $56,000 (12%): circuit breaker = {}", result.is_some());

    if let Some(reason) = result {
        println!("  Reason: {:?}", reason);
    }

    // Position risk check
    let current_oi = Quote::new(dec!(50_000_000));
    let proposed_value = Quote::new(dec!(6_000_000)); // 12% of OI
    let position_result = check_position_risk(proposed_value, current_oi, &risk_state, &params);
    println!("  Position of $6M with $50M OI (12%): {:?}\n", position_result);
}

/// Conditional order demonstration (stop loss, take profit).
fn scenario_9_conditional_orders() {
    println!("Scenario 9: Conditional Orders\n");

    let mut order_book = ConditionalOrderBook::new(MarketId(1));
    let account = AccountId(1);
    let now = Timestamp::from_millis(1000);

    // Stop loss for long position
    let stop_id = order_book.next_id();
    let stop_loss = ConditionalOrder::new_stop_loss(
        stop_id,
        account,
        MarketId(1),
        Side::Long,
        dec!(1.0),
        Price::new_unchecked(dec!(48000)),
        now,
    );
    order_book.insert(stop_loss);
    println!("  Added stop loss @ $48,000 for 1 BTC long");

    // Take profit for the same position
    let tp_id = order_book.next_id();
    let take_profit = ConditionalOrder::new_take_profit(
        tp_id,
        account,
        MarketId(1),
        Side::Long,
        dec!(1.0),
        Price::new_unchecked(dec!(55000)),
        now,
    );
    order_book.insert(take_profit);
    println!("  Added take profit @ $55,000 for 1 BTC long");

    // Check triggers at different prices
    let current_price = Price::new_unchecked(dec!(50000));
    let triggered = order_book.check_triggers(current_price);
    println!("  At $50,000: {} orders triggered", triggered.len());

    let drop_price = Price::new_unchecked(dec!(47500));
    let triggered = order_book.check_triggers(drop_price);
    println!("  At $47,500: {} orders triggered (stop loss)", triggered.len());

    // Remove stop and check take profit
    order_book.remove(stop_id);
    let rise_price = Price::new_unchecked(dec!(56000));
    let triggered = order_book.check_triggers(rise_price);
    println!("  At $56,000: {} orders triggered (take profit)\n", triggered.len());

    let _ = (stop_id, tp_id); // Silence unused warnings
}

/// ADL ranking demonstration.
fn scenario_10_adl_ranking() {
    println!("Scenario 10: ADL Ranking\n");

    let mark_price = Price::new_unchecked(dec!(55000));
    let params = AdlParams::default();

    // Create some positions with varying PnL and leverage
    let positions = vec![
        // High PnL ratio, low leverage
        create_test_position(1, MarketId(1), dec!(1.0), dec!(50000), dec!(10000), Side::Long),
        // Medium PnL ratio, high leverage
        create_test_position(2, MarketId(1), dec!(2.0), dec!(50000), dec!(5000), Side::Long),
        // Low PnL ratio, medium leverage
        create_test_position(3, MarketId(1), dec!(0.5), dec!(50000), dec!(8000), Side::Long),
    ];

    let candidates = rank_adl_candidates(positions.clone(), Side::Long, mark_price);

    println!("  ADL Rankings (highest score first):");
    for (i, c) in candidates.iter().enumerate() {
        println!(
            "    #{}: Account {}, score={:.4}, unrealized PnL=${}",
            i + 1, c.account_id.0, c.score, c.unrealized_pnl
        );
    }

    // Calculate ADL sizes
    let bad_debt = Quote::new(dec!(5000));
    let sizes = calculate_adl_sizes(&candidates, bad_debt, mark_price, &params);

    println!("  ADL sizes to cover $5,000 bad debt:");
    for (acc, size) in &sizes {
        println!("    Account {}: close {} BTC", acc.0, size.abs());
    }
    println!();
}

/// Near margin boundary edge cases.
fn scenario_11_near_margin_edge() {
    println!("Scenario 11: Near Margin Edge Cases\n");

    let mut engine = Engine::new(EngineConfig::default());
    engine.add_market(MarketConfig::btc_perp());
    engine.fund_insurance(Quote::new(dec!(100000)));

    let trader = engine.create_account();
    let counterparty = engine.create_account();

    // Deposit exact amount to be at edge
    engine.deposit(trader, Quote::new(dec!(2500))).unwrap();
    engine.deposit(counterparty, Quote::new(dec!(500000))).unwrap();

    engine
        .update_index_price(MarketId(1), Price::new_unchecked(dec!(50000)))
        .unwrap();

    // Counterparty provides liquidity
    engine
        .place_limit_order(
            counterparty,
            MarketId(1),
            Side::Short,
            dec!(10.0),
            Price::new_unchecked(dec!(50000)),
            TimeInForce::GTC,
        )
        .unwrap();

    // Open position at maximum leverage
    let result = engine.place_market_order(trader, MarketId(1), Side::Long, dec!(1.0));

    match result {
        Ok(order) => {
            println!("  Opened 1 BTC position with $2,500 collateral");
            let pos = engine.get_account(trader).unwrap().get_position(MarketId(1)).unwrap();
            let notional = dec!(50000);
            let effective_leverage = notional / pos.collateral.value();
            println!("  Effective leverage: {:.1}x", effective_leverage);

            // Check what price triggers liquidation using margin params
            let market = engine.get_market(MarketId(1)).unwrap();
            let mm_ratio = market.config.margin_params.maintenance_margin_ratio;
            let liq_price = dec!(50000) * (dec!(1) - pos.collateral.value() / notional + mm_ratio);
            println!("  Estimated liquidation price: ${:.0}", liq_price);

            // Move price just above liquidation
            let near_liq = liq_price + dec!(500);
            engine.update_index_price(MarketId(1), Price::new_unchecked(near_liq)).unwrap();
            let liqs = engine.check_liquidations(MarketId(1)).unwrap();
            println!("  At ${:.0}: liquidated = {}", near_liq, !liqs.is_empty());

            // Move price to liquidation
            let below_liq = liq_price - dec!(100);
            engine.update_index_price(MarketId(1), Price::new_unchecked(below_liq)).unwrap();
            let liqs = engine.check_liquidations(MarketId(1)).unwrap();
            println!("  At ${:.0}: liquidated = {}", below_liq, !liqs.is_empty());

            let _ = order;
        }
        Err(e) => {
            println!("  Order rejected (insufficient margin): {:?}", e);
        }
    }
    println!();
}

/// Helper to create test position for ADL demo.
fn create_test_position(
    account_id: u64,
    market_id: MarketId,
    size: Decimal,
    entry_price: Decimal,
    collateral: Decimal,
    side: Side,
) -> (AccountId, Position) {
    let signed_size = if side == Side::Long {
        SignedSize::new(size)
    } else {
        SignedSize::new(-size)
    };
    let leverage_val = entry_price * size / collateral;
    let pos = Position::new(
        market_id,
        signed_size,
        Price::new_unchecked(entry_price),
        Quote::new(collateral),
        Leverage::new(leverage_val).unwrap_or(Leverage::new(dec!(1)).unwrap()),
        Decimal::ZERO, // funding index
        Timestamp::from_millis(0),
    );
    (AccountId(account_id), pos)
}

/// Price feed aggregation from multiple sources.
fn scenario_12_price_aggregation() {
    println!("Scenario 12: Price Feed Aggregation\n");

    let config = PriceFeedConfig {
        min_sources: 2,
        max_staleness_seconds: 60,
        max_source_deviation: dec!(0.02), // 2%
        use_median: true,
        source_weights: Vec::new(),
    };

    let mut aggregator = PriceAggregator::new(config);

    // simulate multiple price sources
    aggregator.submit_price(PriceUpdate::new(dec!(50100), 1000, 1).with_ttl(60)); // Pyth
    aggregator.submit_price(PriceUpdate::new(dec!(50000), 1000, 2).with_ttl(60)); // Chainlink
    aggregator.submit_price(PriceUpdate::new(dec!(49900), 1000, 3).with_ttl(60)); // Binance

    let agg_price = aggregator.get_price(1030).unwrap();
    println!("  Sources: Pyth=$50,100, Chainlink=$50,000, Binance=$49,900");
    println!("  Aggregated (median): ${}", agg_price.price);

    // test staleness
    let stale_result = aggregator.get_price(1100);
    println!("  At t=1100: {:?}", stale_result.err().unwrap());

    // TWAP calculation
    let mut twap = TwapCalculator::new(300); // 5 minute window
    twap.add_sample(0, dec!(50000));
    twap.add_sample(60, dec!(50500));
    twap.add_sample(120, dec!(51000));
    twap.add_sample(180, dec!(50200));

    println!("  TWAP over 4 samples: ${}\n", twap.get_twap().unwrap());
}

/// Shared liquidity pool operations.
fn scenario_13_liquidity_pool() {
    println!("Scenario 13: Liquidity Pool\n");

    let config = PoolConfig {
        pool_id: 1,
        name: "BTC-PERP LP".to_string(),
        tvl: dec!(10_000_000),
        max_utilization: dec!(0.8),
        fee_bps: 10,
        active: true,
    };

    let mut pool = SharedPool::new(config);

    println!("  Pool TVL: ${}", pool.tvl());
    println!("  Max utilization: 80%");

    // get quote for a trade
    let quote = pool.get_quote(Side::Long, dec!(10), dec!(50000)).unwrap();
    println!("  Quote for 10 BTC long:");
    println!("    Execution price: ${:.2}", quote.price);
    println!("    Fee: ${:.2}", quote.fee);
    println!("    Price impact: {:.4}%", quote.price_impact * dec!(100));

    // execute the trade
    pool.execute_quote(&quote).unwrap();
    println!("  After trade: utilization = {:.4}%", pool.utilization() * dec!(100));

    // check available liquidity
    let (long_liq, short_liq) = pool.available_liquidity();
    println!("  Available liquidity: ${} (each side)\n", long_liq);

    let _ = short_liq;
}

/// Deposit and withdrawal custody flows.
fn scenario_14_custody_flows() {
    println!("Scenario 14: Custody Flows\n");

    let config = CustodyConfig::default();
    let mut custody = CustodyManager::new(config);

    // process a deposit
    let deposit = DepositRequest::new(
        "tx-abc123".to_string(),
        AccountId(1),
        CollateralType::Usdc,
        dec!(10000),
        1000,
    );

    custody.initiate_deposit(deposit).unwrap();
    println!("  Initiated deposit: $10,000 USDC");
    println!("  Pending deposits: {}", custody.pending_deposit_count());

    let confirmed = custody.confirm_deposit(&"tx-abc123".to_string(), 1050).unwrap();
    println!("  Deposit confirmed at t={}", confirmed.confirmed_at.unwrap());
    println!("  Total deposited: ${}", custody.total_deposited());

    // process a withdrawal
    let withdrawal = custody.request_withdrawal(
        AccountId(1),
        CollateralType::Usdc,
        dec!(500),
        "0x1234...abcd".to_string(),
        2000,
        dec!(10000), // available balance
    ).unwrap();

    println!("  Withdrawal requested: ${}, fee: ${}", withdrawal.amount, withdrawal.fee);
    println!("  Net withdrawal: ${}", withdrawal.net_amount());

    custody.process_withdrawal(&withdrawal.tx_id, 2010).unwrap();
    println!("  Total withdrawn: ${}\n", custody.total_withdrawn());
}

/// Settlement batch processing.
fn scenario_15_settlement_batching() {
    println!("Scenario 15: Settlement Batching\n");

    let mut manager = SettlementManager::new(100);
    let mut backend = InMemorySettlement::new();

    // set up initial balances
    backend.set_balance(AccountId(1), dec!(10000));
    backend.set_balance(AccountId(2), dec!(5000));
    backend.set_balance(AccountId(3), dec!(8000));

    // build a settlement batch
    manager.begin_batch(1000);

    manager.add_instruction(SettlementInstruction::Transfer {
        from: AccountId(1),
        to: AccountId(2),
        amount: dec!(100),
        reason: TransferReason::TradeFee,
    }).unwrap();

    manager.add_instruction(SettlementInstruction::FundingPayment {
        payer: AccountId(2),
        receiver: AccountId(3),
        amount: dec!(50),
    }).unwrap();

    manager.add_instruction(SettlementInstruction::RealizePnl {
        account_id: AccountId(3),
        pnl: dec!(200),
        counterparty: AccountId(1),
    }).unwrap();

    let batch_id = manager.commit_batch().unwrap();
    println!("  Created batch #{} with 3 instructions", batch_id);

    // execute the batch
    let batch = manager.next_pending().unwrap();
    let commitment = backend.execute(&batch).unwrap();
    println!("  Executed batch, commitment: {}", commitment);

    // check final balances
    println!("  Final balances:");
    println!("    Account 1: ${} (was $10,000)", backend.get_balance(AccountId(1)));
    println!("    Account 2: ${} (was $5,000)", backend.get_balance(AccountId(2)));
    println!("    Account 3: ${} (was $8,000)\n", backend.get_balance(AccountId(3)));
}

/// Configuration presets for different environments.
fn scenario_16_config_presets() {
    println!("Scenario 16: Configuration Presets\n");

    // test different environment configs
    let dev = IntegrationConfig::default();
    let testnet = IntegrationConfig::testnet();
    let mainnet = IntegrationConfig::mainnet_conservative();

    println!("  Development config:");
    println!("    Max leverage: {}x", dev.max_leverage());
    println!("    Maker fee: {} bps", dev.fees.maker_fee_bps);

    println!("  Testnet config:");
    println!("    Max leverage: {}x", testnet.max_leverage());
    println!("    Maker fee: {} bps (free!)", testnet.fees.maker_fee_bps);

    println!("  Mainnet config:");
    println!("    Max leverage: {}x", mainnet.max_leverage());
    println!("    Min price sources: {}", mainnet.price_feed.min_sources);

    // validate API command
    let cmd = EngineCommand::PlaceOrder {
        account_id: AccountId(1),
        side: Side::Long,
        size: dec!(1.0),
        limit_price: Some(dec!(50000)),
        post_only: false,
        fill_or_kill: false,
        client_order_id: Some("my-order".to_string()),
    };

    let validation = validate_command(&cmd);
    println!("  API command validation: {:?}", validation.is_ok());

    // show environment helper
    let env_config = Environment::Mainnet.config();
    println!("  Environment::Mainnet max leverage: {}x\n", env_config.max_leverage());
}
