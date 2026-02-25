pub mod fees_test;
pub mod interest_rate_test;
pub mod liquidate_test;
pub mod oracle_test;
pub mod risk_params_test;
pub mod storage_test;
pub mod test;
pub mod test_helpers;
pub mod withdraw_test;
// Cross-asset tests disabled - contract methods not yet implemented
pub mod governance_test;
pub mod views_test;
// Cross-asset tests re-enabled when contract exposes full CA API (try_* return Result; get_user_asset_position; try_ca_repay_debt)
// pub mod test_cross_asset;
pub mod bridge_test;
