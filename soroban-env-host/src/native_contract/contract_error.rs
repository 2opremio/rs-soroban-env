use num_derive::FromPrimitive;
use soroban_env_common::Status;

// Use the same error for all the built-in contract error.
// In theory we could have a separate enum for each built-in contract, but it's
// not clear how to distinguish them if multiple built-in contracts are involved.
#[derive(Debug, FromPrimitive, PartialEq, Eq)]
pub enum ContractError {
    InternalError = 1,
    OperationNotSupportedError = 2,
    AlreadyInitializedError = 3,

    UnauthorizedError = 4,
    AuthenticationError = 5,
    AccountMissingError = 6,
    AccountIsNotClassic = 7,

    NegativeAmountError = 8,
    AllowanceError = 9,
    BalanceError = 10,
    BalanceDeauthorizedError = 11,
    OverflowError = 12,
    TrustlineMissingError = 13,
}

impl From<ContractError> for Status {
    fn from(err: ContractError) -> Self {
        Status::from_contract_error(err as u32)
    }
}
