# Token Receiver Hook Documentation

## Overview

The `receive` hook allows the StellarLend contract to automatically handle incoming token transfers for collateral deposits and debt repayments. This enables a more seamless user experience where users can simply send tokens to the contract with a specific payload to trigger protocol actions.

## Function Signature

```rust
pub fn receive(
    env: Env,
    token_asset: Address,
    from: Address,
    amount: i128,
    payload: Vec<Val>,
) -> Result<(), BorrowError>
```

## Parameters

- `env`: The contract environment
- `token_asset`: The address of the asset contract (this should be validated for security)
- `from`: The address of the token sender
- `amount`: The amount of tokens transferred
- `payload`: A vector of values, where the first element is a `Symbol` indicating the action (`deposit` or `repay`)

## Actions

### Deposit

To deposit collateral via a token transfer, the user should provide a payload containing the symbol `"deposit"`.

**Mechanism**:
1. Token contract calls `receive` on the lending contract.
2. Lending contract validates the action and updates the user's `CollateralPosition`.
3. Emits a `deposit` event.

### Repay

To repay debt via a token transfer, the user should provide a payload containing the symbol `"repay"`.

**Mechanism**:
1. Token contract calls `receive` on the lending contract.
2. Lending contract calculates accrued interest and updates the user's `DebtPosition`.
3. Interest is repaid first, then principal.
4. Updates protocol-wide `TotalDebt`.
5. Emits a `repay` event.

## Security Considerations

1. **Caller Validation**: While the simplified interface takes `token_asset` as an argument, a production implementation should verify that the actual caller is indeed the token contract being reported.
2. **Reentrancy**: Soroban provides protection against reentrancy, but the implementation follows best practices by updating state before emitting events.
3. **Payload Sanitization**: The hook validates that the payload contains a valid action before proceeding.
4. **Unauthorized Access**: The hook relies on the token contract's authorization mechanism for the initial transfer.

## Usage Example

### Via Token Transfer

```rust
// User sends 100 USDC to LendingContract with "deposit" payload
token_client.transfer(
    &user,
    &lending_contract_id,
    &100_000_000,
    &vec![&env, symbol_short!("deposit").into_val(&env)]
);
```

### Direct Call (Alternative)

The contract also exposes direct `deposit` and `repay` functions for flexibility.

```rust
lending_contract_client.deposit(&user, &usdc_asset, &100_000_000);
```
