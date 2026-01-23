//! Perpetual DEX Core Simulation
//!
//! Week 1: Core math validation and stress testing

use perps_core::*;
use rust_decimal_macros::dec;

fn main() {
    println!("=== Perpetual DEX Core - Week 1 Simulation ===\n");

    // Initialize parameters
    let margin_params = MarginParams::default();
    let funding_params = FundingParams::default();
    let _mark_params = MarkPriceParams::default();
    let liq_params = LiquidationParams::default();

    // Scenario: Open long position, price moves, check margin/liquidation
    simulate_long_position_lifecycle(&margin_params, &funding_params);
    println!("\n{}", "=".repeat(60));
    simulate_leverage_tiers(&margin_params);
    println!("\n{}", "=".repeat(60));
    simulate_funding_mechanics(&funding_params);
    println!("\n{}", "=".repeat(60));
    simulate_liquidation_cascade(&margin_params, &liq_params);
}

fn simulate_long_position_lifecycle(margin_params: &MarginParams, _funding_params: &FundingParams) {
    println!("📈 Scenario: Long Position Lifecycle\n");

    let entry_price = Price::new_unchecked(dec!(50000));
    let size = SignedSize::new(dec!(1)); // 1 BTC long
    let leverage = Leverage::new(dec!(10)).unwrap();

    // Calculate margin requirements
    let margin_req = calculate_margin_requirement(size, entry_price, leverage, margin_params);
    println!("Entry: {} @ ${}", size, entry_price);
    println!("Leverage: {}", leverage);
    println!("Initial Margin Required: ${}", margin_req.initial);
    println!("Maintenance Margin: ${}", margin_req.maintenance);

    // Create position
    let mut account = Account::new(AccountId(1), Timestamp::from_millis(0));
    account.deposit(Quote::new(dec!(10000)));

    let position = Position::new(
        MarketId(1),
        size,
        entry_price,
        margin_req.initial,
        leverage,
        dec!(0),
        Timestamp::from_millis(0),
    );

    println!("\n--- Price moves up 4% ---");
    let new_price = Price::new_unchecked(dec!(52000));
    let pnl = position.unrealized_pnl(new_price);
    let equity = position.equity(new_price, dec!(0));
    println!("Mark Price: ${}", new_price);
    println!("Unrealized PnL: ${}", pnl);
    println!("Position Equity: ${}", equity);

    let status = evaluate_margin_status(equity, &margin_req);
    println!("Margin Status: {:?}", status);

    println!("\n--- Price drops 10% from entry ---");
    let bad_price = Price::new_unchecked(dec!(45000));
    let bad_pnl = position.unrealized_pnl(bad_price);
    let bad_equity = position.equity(bad_price, dec!(0));
    println!("Mark Price: ${}", bad_price);
    println!("Unrealized PnL: ${}", bad_pnl);
    println!("Position Equity: ${}", bad_equity);

    let bad_status = evaluate_margin_status(bad_equity, &margin_req);
    println!("Margin Status: {:?}", bad_status);

    // Check liquidation status
    let notional = notional_value(size, bad_price);
    let liq_status = evaluate_liquidation(
        bad_equity,
        &margin_req,
        notional,
        entry_price,
        bad_price,
        Side::Long,
    );
    println!("Liquidation Status: {:?}", liq_status);
}

fn simulate_leverage_tiers(margin_params: &MarginParams) {
    println!("📊 Scenario: Dynamic Leverage Tiers\n");

    let price = Price::new_unchecked(dec!(50000));
    let max_leverage = Leverage::new(dec!(50)).unwrap();

    let test_sizes = [
        (dec!(1), "Small ($50k)"),
        (dec!(5), "Medium ($250k)"),
        (dec!(20), "Large ($1M)"),
        (dec!(100), "Whale ($5M)"),
    ];

    for (size_val, label) in &test_sizes {
        let size = SignedSize::new(*size_val);
        let notional = notional_value(size, price);
        let effective_lev = effective_max_leverage(notional, margin_params);
        let margin_req = calculate_margin_requirement(size, price, max_leverage, margin_params);

        println!(
            "{}: Notional ${}, Max Leverage {}, IM ${}",
            label,
            notional,
            effective_lev,
            margin_req.initial
        );
    }
}

fn simulate_funding_mechanics(funding_params: &FundingParams) {
    println!("💰 Scenario: Funding Rate Mechanics\n");

    let index_price = Price::new_unchecked(dec!(50000));

    // Premium scenarios
    let premiums = [
        (dec!(50500), "0.5% premium"),
        (dec!(49500), "0.5% discount"),
        (dec!(52500), "5% premium (extreme)"),
    ];

    for (mark_val, label) in &premiums {
        let mark_price = Price::new_unchecked(*mark_val);
        let premium = calculate_premium_index(mark_price, index_price);
        let funding_rate = calculate_funding_rate(premium, funding_params);
        let annual_rate = annualized_funding_rate(funding_rate);

        println!("{}: Premium {:.4}%, Rate {:.4}%, APR {:.2}%",
            label,
            premium * dec!(100),
            funding_rate * dec!(100),
            annual_rate * dec!(100)
        );
    }

    println!("\n--- Funding Payment Example ---");
    let size = SignedSize::new(dec!(1)); // 1 BTC long
    let mark = Price::new_unchecked(dec!(50500));
    let funding_rate = dec!(0.001); // 0.1%

    let payment = calculate_funding_payment(size, mark, funding_rate);
    println!("Long 1 BTC @ 0.1% funding: pays ${}", payment);

    let short_payment = calculate_funding_payment(SignedSize::new(dec!(-1)), mark, funding_rate);
    println!("Short 1 BTC @ 0.1% funding: receives ${}", short_payment.abs());
}

fn simulate_liquidation_cascade(margin_params: &MarginParams, liq_params: &LiquidationParams) {
    println!("⚠️  Scenario: Liquidation Cascade\n");

    // Setup: Multiple positions with different leverage
    let positions = vec![
        ("Conservative", dec!(2), Leverage::new(dec!(5)).unwrap()),
        ("Moderate", dec!(1), Leverage::new(dec!(10)).unwrap()),
        ("Aggressive", dec!(0.5), Leverage::new(dec!(20)).unwrap()),
    ];

    let entry_price = Price::new_unchecked(dec!(50000));

    println!("Initial positions at $50,000:\n");
    for (name, size_val, leverage) in &positions {
        let size = SignedSize::new(*size_val);
        let margin_req = calculate_margin_requirement(size, entry_price, *leverage, margin_params);
        let mmf = margin_req.maintenance.value() / (size.abs() * entry_price.value());
        let liq_price = calculate_liquidation_price(entry_price, *leverage, Side::Long, mmf);
        
        println!(
            "  {}: {} BTC @ {}, IM ${}, Liq Price ~${}",
            name, size_val, leverage, margin_req.initial, liq_price
        );
    }

    // Price crash simulation
    println!("\n--- Price crashes to $42,000 (16% drop) ---\n");
    let crash_price = Price::new_unchecked(dec!(42000));

    for (name, size_val, leverage) in &positions {
        let size = SignedSize::new(*size_val);
        let margin_req = calculate_margin_requirement(size, entry_price, *leverage, margin_params);
        
        // Create simulated position
        let position = Position::new(
            MarketId(1),
            size,
            entry_price,
            margin_req.initial,
            *leverage,
            dec!(0),
            Timestamp::from_millis(0),
        );

        let pnl = position.unrealized_pnl(crash_price);
        let equity = position.equity(crash_price, dec!(0));
        let notional = notional_value(size, crash_price);
        
        let status = evaluate_liquidation(
            equity,
            &margin_req,
            notional,
            entry_price,
            crash_price,
            Side::Long,
        );

        let status_str = match &status {
            LiquidationStatus::Safe { .. } => "✅ Safe",
            LiquidationStatus::AtRisk { buffer_percent, .. } => 
                &format!("⚠️  At Risk ({:.1}% buffer)", buffer_percent),
            LiquidationStatus::Liquidatable { shortfall, .. } => 
                &format!("🔴 LIQUIDATABLE (${} shortfall)", shortfall),
            LiquidationStatus::Bankrupt { bad_debt } => 
                &format!("💀 BANKRUPT (${} bad debt)", bad_debt),
        };

        println!(
            "  {}: PnL ${}, Equity ${} -> {}",
            name, pnl, equity, status_str
        );
    }

    // Liquidation penalty calculation
    println!("\n--- Liquidation Penalties ---");
    let liq_position_value = Quote::new(dec!(50000));
    let penalty = calculate_liquidation_penalty(liq_position_value, liq_params);
    println!("For $50,000 position:");
    println!("  Total Penalty: ${}", penalty.total);
    println!("  Liquidator Reward: ${}", penalty.liquidator_reward);
    println!("  Insurance Fund: ${}", penalty.insurance_contribution);
}

