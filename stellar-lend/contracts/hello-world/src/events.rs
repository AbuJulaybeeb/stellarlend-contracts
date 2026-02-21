/// # StellarLend Protocol – Event Logging
///
/// Defines a **consistent, structured event schema** for every state-changing
/// action in the StellarLend protocol.
///
/// ## Design principles
/// - Each event is its own `#[contractevent]` struct. The macro auto-derives
///   the lowercase snake_case struct name as the leading topic, generates XDR
///   spec entries, and exposes a `.publish(&env)` method.
/// - Fields annotated with `#[topic]` become additional Soroban event topics.
///   All other fields are packed into the event data payload (default format: map).
/// - `emit_*` helper functions wrap struct construction and call `.publish`,
///   providing a single call-site per action.
/// - **No sensitive data**: all fields are publicly observable state only
///   (`Address`, `Symbol`, `i128`, `u32`, `u64`, `bool`, `Option<Address>`).
///
/// ## Off-chain indexing
/// Events are indexed by contract address + the auto-generated topic (the
/// snake_case struct name). Consumers retrieve them via Stellar Horizon or a
/// Soroban event streaming service.
use soroban_sdk::{contractevent, Address, Env, Symbol};

// ─────────────────────────────────────────────────────────────────────────────
// Protocol action event structs
// ─────────────────────────────────────────────────────────────────────────────

/// Emitted when a user deposits collateral into the protocol.
///
/// # Fields
/// * `user` – The depositor's address.
/// * `asset` – The deposited asset; `None` for native XLM.
/// * `amount` – The deposit amount in the asset's smallest unit.
/// * `timestamp` – Ledger timestamp at deposit time.
///
/// # Security
/// Only the actor's own publicly observable deposit data is recorded.
#[contractevent]
#[derive(Clone, Debug)]
pub struct DepositEvent {
    pub user: Address,
    pub asset: Option<Address>,
    pub amount: i128,
    pub timestamp: u64,
}

/// Emitted when a user withdraws collateral from the protocol.
///
/// # Fields
/// * `user` – The withdrawer's address.
/// * `asset` – The withdrawn asset; `None` for native XLM.
/// * `amount` – The withdrawal amount in the asset's smallest unit.
/// * `timestamp` – Ledger timestamp at withdrawal time.
#[contractevent]
#[derive(Clone, Debug)]
pub struct WithdrawalEvent {
    pub user: Address,
    pub asset: Option<Address>,
    pub amount: i128,
    pub timestamp: u64,
}

/// Emitted when a user borrows assets from the protocol.
///
/// # Fields
/// * `user` – The borrower's address.
/// * `asset` – The borrowed asset; `None` for native XLM.
/// * `amount` – The borrowed amount in the asset's smallest unit.
/// * `timestamp` – Ledger timestamp at borrow time.
#[contractevent]
#[derive(Clone, Debug)]
pub struct BorrowEvent {
    pub user: Address,
    pub asset: Option<Address>,
    pub amount: i128,
    pub timestamp: u64,
}

/// Emitted when a user repays debt to the protocol.
///
/// # Fields
/// * `user` – The repayer's address.
/// * `asset` – The repaid asset; `None` for native XLM.
/// * `amount` – The total amount repaid.
/// * `timestamp` – Ledger timestamp at repayment time.
#[contractevent]
#[derive(Clone, Debug)]
pub struct RepayEvent {
    pub user: Address,
    pub asset: Option<Address>,
    pub amount: i128,
    pub timestamp: u64,
}

/// Emitted when a liquidator liquidates an undercollateralised position.
///
/// # Fields
/// * `liquidator` – The liquidator's address.
/// * `borrower` – The address of the position being liquidated.
/// * `debt_asset` – The debt asset; `None` for native XLM.
/// * `collateral_asset` – The collateral seized; `None` for native XLM.
/// * `debt_liquidated` – The debt amount repaid by the liquidator.
/// * `collateral_seized` – The collateral transferred to the liquidator.
/// * `incentive_amount` – The liquidation bonus (in collateral terms).
/// * `timestamp` – Ledger timestamp at liquidation time.
///
/// # Security
/// Both liquidator and borrower are public actors.
/// No private data of uninvolved users is disclosed.
#[contractevent]
#[derive(Clone, Debug)]
pub struct LiquidationEvent {
    pub liquidator: Address,
    pub borrower: Address,
    pub debt_asset: Option<Address>,
    pub collateral_asset: Option<Address>,
    pub debt_liquidated: i128,
    pub collateral_seized: i128,
    pub incentive_amount: i128,
    pub timestamp: u64,
}

/// Emitted when a flash loan is initiated.
///
/// # Fields
/// * `user` – The flash loan borrower's address.
/// * `asset` – The borrowed asset.
/// * `amount` – The principal.
/// * `fee` – The fee charged.
/// * `callback` – The callback contract responsible for repayment.
/// * `timestamp` – Ledger timestamp at initiation.
#[contractevent]
#[derive(Clone, Debug)]
pub struct FlashLoanInitiatedEvent {
    pub user: Address,
    pub asset: Address,
    pub amount: i128,
    pub fee: i128,
    pub callback: Address,
    pub timestamp: u64,
}

/// Emitted when a flash loan is successfully repaid.
///
/// # Fields
/// * `user` – The repayer's address.
/// * `asset` – The repaid asset.
/// * `amount` – The principal repaid.
/// * `fee` – The fee repaid.
/// * `timestamp` – Ledger timestamp at repayment.
#[contractevent]
#[derive(Clone, Debug)]
pub struct FlashLoanRepaidEvent {
    pub user: Address,
    pub asset: Address,
    pub amount: i128,
    pub fee: i128,
    pub timestamp: u64,
}

/// Emitted for generic admin-initiated state-changing actions.
///
/// # Fields
/// * `actor` – The admin's address.
/// * `action` – A symbol identifying the action (e.g. `"initialize"`).
/// * `timestamp` – Ledger timestamp of the action.
///
/// # Security
/// Only the public admin address is recorded; no credentials exposed.
#[contractevent]
#[derive(Clone, Debug)]
pub struct AdminActionEvent {
    pub actor: Address,
    pub action: Symbol,
    pub timestamp: u64,
}

/// Emitted when an oracle price is updated.
///
/// # Fields
/// * `actor` – The address that submitted the price update.
/// * `asset` – The asset whose price was updated.
/// * `price` – The new price (in oracle's native units).
/// * `decimals` – Number of decimal places for the price.
/// * `oracle` – The oracle contract address.
/// * `timestamp` – Ledger timestamp at update time.
#[contractevent]
#[derive(Clone, Debug)]
pub struct PriceUpdatedEvent {
    pub actor: Address,
    pub asset: Address,
    pub price: i128,
    pub decimals: u32,
    pub oracle: Address,
    pub timestamp: u64,
}

/// Emitted when risk parameters are updated by an admin.
///
/// # Fields
/// * `actor` – The admin's address.
/// * `timestamp` – Ledger timestamp of the update.
///
/// Note: individual parameter values can be queried from contract state.
#[contractevent]
#[derive(Clone, Debug)]
pub struct RiskParamsUpdatedEvent {
    pub actor: Address,
    pub timestamp: u64,
}

/// Emitted when the pause state of any protocol operation changes.
///
/// # Fields
/// * `actor` – The admin's address.
/// * `operation` – Symbol for the paused/unpaused operation
///   (e.g. `"pause_deposit"`, `"pause_borrow"`, `"emergency"`).
/// * `paused` – `true` if paused, `false` if unpaused.
/// * `timestamp` – Ledger timestamp of the change.
#[contractevent]
#[derive(Clone, Debug)]
pub struct PauseStateChangedEvent {
    pub actor: Address,
    pub operation: Symbol,
    pub paused: bool,
    pub timestamp: u64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Emitter helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Emit a deposit event.
/// Call this after successfully updating collateral storage.
pub fn emit_deposit(e: &Env, event: DepositEvent) {
    event.publish(e);
}

/// Emit a withdrawal event.
/// Call this after successfully updating collateral storage.
pub fn emit_withdrawal(e: &Env, event: WithdrawalEvent) {
    event.publish(e);
}

/// Emit a borrow event.
/// Call this after successfully updating debt storage.
pub fn emit_borrow(e: &Env, event: BorrowEvent) {
    event.publish(e);
}

/// Emit a repay event.
/// Call this after successfully reducing debt storage.
pub fn emit_repay(e: &Env, event: RepayEvent) {
    event.publish(e);
}

/// Emit a liquidation event.
/// Call this after the debt repayment and collateral seizure are committed.
pub fn emit_liquidation(e: &Env, event: LiquidationEvent) {
    event.publish(e);
}

/// Emit a flash-loan-initiated event.
/// Call this after the flash loan record is stored and tokens transferred.
pub fn emit_flash_loan_initiated(e: &Env, event: FlashLoanInitiatedEvent) {
    event.publish(e);
}

/// Emit a flash-loan-repaid event.
/// Call this after the record is cleared and repayment received.
pub fn emit_flash_loan_repaid(e: &Env, event: FlashLoanRepaidEvent) {
    event.publish(e);
}

/// Emit an admin-action event.
/// Use for initialization or admin operations without a dedicated event type.
pub fn emit_admin_action(e: &Env, event: AdminActionEvent) {
    event.publish(e);
}

/// Emit a price-updated event.
/// Call this after committing a new oracle price to storage.
pub fn emit_price_updated(e: &Env, event: PriceUpdatedEvent) {
    event.publish(e);
}

/// Emit a risk-params-updated event.
/// Call this after risk configuration has been written to storage.
pub fn emit_risk_params_updated(e: &Env, event: RiskParamsUpdatedEvent) {
    event.publish(e);
}

/// Emit a pause-state-changed event.
/// Call this after any pause switch (including emergency) is toggled.
pub fn emit_pause_state_changed(e: &Env, event: PauseStateChangedEvent) {
    event.publish(e);
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct PositionUpdatedEvent {
    pub user: Address,
    pub collateral: i128,
    pub debt: i128,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct AnalyticsUpdatedEvent {
    pub user: Address,
    pub activity_type: soroban_sdk::String,
    pub amount: i128,
    pub timestamp: u64,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct UserActivityTrackedEvent {
    pub user: Address,
    pub operation: Symbol,
    pub amount: i128,
    pub timestamp: u64,
}

pub fn emit_position_updated(e: &Env, event: PositionUpdatedEvent) {
    event.publish(e);
}

pub fn emit_analytics_updated(e: &Env, event: AnalyticsUpdatedEvent) {
    event.publish(e);
}

pub fn emit_user_activity_tracked(e: &Env, event: UserActivityTrackedEvent) {
    event.publish(e);
}
