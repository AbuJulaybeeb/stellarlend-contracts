pub mod edge_cases_test;
pub mod events_test;
pub mod integration_test;
pub mod interest_rate_test;
pub mod liquidate_test;
pub mod oracle_test;
pub mod risk_params_test;
pub mod security_test;
pub mod test;
// Cross-asset tests re-enabled when contract exposes full CA API (try_* return Result; get_user_asset_position; try_ca_repay_debt)
// pub mod test_cross_asset;
