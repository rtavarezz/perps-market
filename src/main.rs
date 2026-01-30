//! Perpetual DEX Core Simulation.
//!
//! Demonstrates the full trading engine lifecycle including order matching,
//! position tracking, funding settlement, and liquidation cascades.

use perps_core::*;
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

