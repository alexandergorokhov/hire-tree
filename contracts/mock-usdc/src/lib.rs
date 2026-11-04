//! mock-usdc: FAKE test USD for Hire Tree. NEP-141 + storage management.
//!
//! IT HAS NO VALUE. It exists so sandbox and testnet runs can move
//! "dollars" without real money. 6 decimals, like USDC. Only the owner can
//! mint. The metadata name and symbol say FAKE so no wallet shows it as a
//! real stablecoin.

use near_contract_standards::fungible_token::metadata::{
    FungibleTokenMetadata, FungibleTokenMetadataProvider, FT_METADATA_SPEC,
};
use near_contract_standards::fungible_token::{
    FungibleToken, FungibleTokenCore, FungibleTokenResolver,
};
use near_contract_standards::storage_management::{
    StorageBalance, StorageBalanceBounds, StorageManagement,
};
use near_sdk::json_types::U128;
use near_sdk::{
    env, log, near, require, AccountId, BorshStorageKey, NearToken, PanicOnDefault, PromiseOrValue,
};

pub const DECIMALS: u8 = 6;
/// One fake dollar in base units.
pub const ONE: u128 = 1_000_000;

#[derive(BorshStorageKey)]
#[near]
enum Key {
    Token,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct MockUsdc {
    token: FungibleToken,
    owner: AccountId,
}

#[near]
impl MockUsdc {
    /// Only the token account itself may initialise it (deploy + init in one
    /// batch), so nobody can front-run the init and pick the owner.
    #[init]
    #[private]
    pub fn new(owner_id: AccountId) -> Self {
        Self {
            token: FungibleToken::new(Key::Token),
            owner: owner_id,
        }
    }

    /// Mint FAKE test USD (no value). Owner only. Registers the receiver if
    /// needed (the token account pays that storage).
    pub fn mint(&mut self, account_id: AccountId, amount: U128) {
        require!(
            env::predecessor_account_id() == self.owner,
            "only the owner can mint fake test USD"
        );
        require!(amount.0 > 0, "amount must be greater than zero");
        if !self.token.accounts.contains_key(&account_id) {
            self.token.internal_register_account(&account_id);
        }
        self.token.internal_deposit(&account_id, amount.0);
        log!(
            "mint {} fake test USD base units to {}",
            amount.0,
            account_id
        );
    }

    pub fn owner(&self) -> AccountId {
        self.owner.clone()
    }
}

#[near]
impl FungibleTokenCore for MockUsdc {
    #[payable]
    fn ft_transfer(&mut self, receiver_id: AccountId, amount: U128, memo: Option<String>) {
        self.token.ft_transfer(receiver_id, amount, memo)
    }

    #[payable]
    fn ft_transfer_call(
        &mut self,
        receiver_id: AccountId,
        amount: U128,
        memo: Option<String>,
        msg: String,
    ) -> PromiseOrValue<U128> {
        self.token.ft_transfer_call(receiver_id, amount, memo, msg)
    }

    fn ft_total_supply(&self) -> U128 {
        self.token.ft_total_supply()
    }

    fn ft_balance_of(&self, account_id: AccountId) -> U128 {
        self.token.ft_balance_of(account_id)
    }
}

#[near]
impl FungibleTokenResolver for MockUsdc {
    #[private]
    fn ft_resolve_transfer(
        &mut self,
        sender_id: AccountId,
        receiver_id: AccountId,
        amount: U128,
    ) -> U128 {
        let (used, _burned) =
            self.token
                .internal_ft_resolve_transfer(&sender_id, receiver_id, amount);
        used.into()
    }
}

#[near]
impl StorageManagement for MockUsdc {
    #[payable]
    fn storage_deposit(
        &mut self,
        account_id: Option<AccountId>,
        registration_only: Option<bool>,
    ) -> StorageBalance {
        self.token.storage_deposit(account_id, registration_only)
    }

    #[payable]
    fn storage_withdraw(&mut self, amount: Option<NearToken>) -> StorageBalance {
        self.token.storage_withdraw(amount)
    }

    #[payable]
    fn storage_unregister(&mut self, force: Option<bool>) -> bool {
        // Never burn balances: refuse forced unregister of a non-empty account.
        require!(
            force != Some(true),
            "forced unregister is disabled on mock-usdc"
        );
        self.token.internal_storage_unregister(force).is_some()
    }

    fn storage_balance_bounds(&self) -> StorageBalanceBounds {
        self.token.storage_balance_bounds()
    }

    fn storage_balance_of(&self, account_id: AccountId) -> Option<StorageBalance> {
        self.token.storage_balance_of(account_id)
    }
}

#[near]
impl FungibleTokenMetadataProvider for MockUsdc {
    fn ft_metadata(&self) -> FungibleTokenMetadata {
        FungibleTokenMetadata {
            spec: FT_METADATA_SPEC.to_string(),
            name: "FAKE test USD (no value, Hire Tree demo)".to_string(),
            symbol: "fUSD-FAKE".to_string(),
            icon: None,
            reference: None,
            reference_hash: None,
            decimals: DECIMALS,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::test_utils::{accounts, VMContextBuilder};
    use near_sdk::testing_env;

    fn ctx(predecessor: AccountId, deposit: NearToken) {
        testing_env!(VMContextBuilder::new()
            .current_account_id("usdc.test".parse().unwrap())
            .predecessor_account_id(predecessor)
            .attached_deposit(deposit)
            .build());
    }

    fn setup() -> MockUsdc {
        ctx("usdc.test".parse().unwrap(), NearToken::from_yoctonear(0));
        MockUsdc::new(accounts(0))
    }

    #[test]
    fn metadata_says_fake_and_six_decimals() {
        let c = setup();
        let m = c.ft_metadata();
        assert_eq!(m.decimals, 6);
        assert!(m.name.contains("FAKE"));
        assert!(m.symbol.contains("FAKE"));
    }

    #[test]
    fn owner_mints_and_registers() {
        let mut c = setup();
        ctx(accounts(0), NearToken::from_yoctonear(0));
        c.mint(accounts(1), U128(5 * ONE));
        assert_eq!(c.ft_balance_of(accounts(1)).0, 5 * ONE);
        assert_eq!(c.ft_total_supply().0, 5 * ONE);
        c.mint(accounts(1), U128(ONE));
        assert_eq!(c.ft_balance_of(accounts(1)).0, 6 * ONE);
    }

    #[test]
    #[should_panic(expected = "only the owner can mint")]
    fn stranger_cannot_mint() {
        let mut c = setup();
        ctx(accounts(1), NearToken::from_yoctonear(0));
        c.mint(accounts(1), U128(ONE));
    }

    #[test]
    #[should_panic(expected = "amount must be greater than zero")]
    fn zero_mint_refused() {
        let mut c = setup();
        ctx(accounts(0), NearToken::from_yoctonear(0));
        c.mint(accounts(1), U128(0));
    }

    #[test]
    fn transfer_moves_balance() {
        let mut c = setup();
        ctx(accounts(0), NearToken::from_yoctonear(0));
        c.mint(accounts(1), U128(5 * ONE));
        c.mint(accounts(2), U128(1));
        ctx(accounts(1), NearToken::from_yoctonear(1));
        c.ft_transfer(accounts(2), U128(2 * ONE), None);
        assert_eq!(c.ft_balance_of(accounts(1)).0, 3 * ONE);
        assert_eq!(c.ft_balance_of(accounts(2)).0, 2 * ONE + 1);
    }

    #[test]
    fn storage_deposit_registers() {
        let mut c = setup();
        let min = c.storage_balance_bounds().min;
        ctx(accounts(1), min);
        c.storage_deposit(Some(accounts(3)), Some(true));
        assert!(c.storage_balance_of(accounts(3)).is_some());
        assert_eq!(c.ft_balance_of(accounts(3)).0, 0);
    }

    #[test]
    #[should_panic(expected = "forced unregister is disabled")]
    fn forced_unregister_refused() {
        let mut c = setup();
        ctx(accounts(1), NearToken::from_yoctonear(1));
        c.storage_unregister(Some(true));
    }
}
