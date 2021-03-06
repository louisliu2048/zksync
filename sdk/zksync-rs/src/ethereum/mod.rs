//! Utilities for the on-chain operations, such as `Deposit` and `FullExit`.

use eth_client::ETHClient;
use futures::compat::Future01CompatExt;
use models::{
    abi,
    node::{AccountId, TokenLike},
};
use std::str::FromStr;
use web3::contract::tokens::Tokenize;
use web3::contract::{Contract, Options};
use web3::transports::{EventLoopHandle, Http};
use web3::types::{H160, H256, U256};
use web3::Web3;

use crate::{error::ClientError, provider::Provider, tokens_cache::TokensCache, types::Network};

const IERC20_INTERFACE: &str = include_str!("abi/IERC20.json");

impl Network {
    pub fn chain_id(self) -> u8 {
        match self {
            Network::Mainnet => 1,
            Network::Ropsten => 3,
            Network::Rinkeby => 4,
            Network::Localhost => 9,
            Network::Unknown => panic!("Attempt to connect to an unknown network"),
        }
    }
}

/// `EthereumProvider` gains access to on-chain operations, such as deposits and full exits.
/// Methods to interact with Ethereum return corresponding Ethereum transaction hash.
/// In order to monitor transaction execution, an Etherereum node `web3` API is exposed
/// via `EthereumProvider::web3` method.
pub struct EthereumProvider {
    tokens_cache: TokensCache,
    eth_client: ETHClient<Http>,
    erc20_abi: ethabi::Contract,
    // We have to prevent handle from drop, since it will cause event loop termination.
    _event_loop: EventLoopHandle,
}

impl EthereumProvider {
    /// Creates a new Ethereum provider.
    pub async fn new(
        provider: &Provider,
        tokens_cache: TokensCache,
        eth_web3_url: impl AsRef<str>,
        eth_private_key: H256,
        eth_addr: H160,
    ) -> Result<Self, ClientError> {
        let (_event_loop, transport) = Http::new(eth_web3_url.as_ref())
            .map_err(|err| ClientError::NetworkError(err.to_string()))?;

        let network = provider.network;

        let address_response = provider.contract_address().await?;
        let contract_address = if address_response.main_contract.starts_with("0x") {
            &address_response.main_contract[2..]
        } else {
            &address_response.main_contract
        };

        let eth_client = ETHClient::new(
            transport,
            abi::zksync_contract(),
            eth_addr,
            eth_private_key,
            contract_address
                .parse()
                .map_err(|err| ClientError::MalformedResponse(format!("{}", err)))?,
            network.chain_id(),
            1.5f64,
        );

        let abi_string = serde_json::Value::from_str(IERC20_INTERFACE)
            .expect("Malformed IERC20 file")
            .get("abi")
            .expect("Malformed IERC20 file")
            .to_string();
        let erc20_abi = ethabi::Contract::load(abi_string.as_bytes()).unwrap();

        Ok(Self {
            eth_client,
            erc20_abi,
            tokens_cache,
            _event_loop,
        })
    }

    /// Exposes Ethereum node `web3` API.
    pub fn web3(&self) -> &Web3<Http> {
        &self.eth_client.web3
    }

    /// Returns the zkSync contract address.
    pub fn contract_address(&self) -> H160 {
        self.eth_client.contract_addr
    }

    /// Returns the pending nonce for the Ethereum account.
    pub async fn nonce(&self) -> Result<U256, ClientError> {
        self.eth_client
            .pending_nonce()
            .await
            .map_err(|err| ClientError::NetworkError(err.to_string()))
    }

    /// Checks whether ERC20 of a certain token deposit is approved for account.
    pub async fn is_erc20_deposit_approved(
        &self,
        token: impl Into<TokenLike>,
    ) -> Result<bool, ClientError> {
        let token = token.into();
        let erc20_approve_threshold: U256 =
            "57896044618658097711785492504343953926634992332820282019728792003956564819968"
                .parse()
                .unwrap(); // 2^255

        let token = self
            .tokens_cache
            .resolve(token)
            .ok_or(ClientError::UnknownToken)?;

        let contract = Contract::new(
            self.eth_client.web3.eth(),
            token.address,
            self.erc20_abi.clone(),
        );

        let query = contract.query(
            "allowance",
            (self.eth_client.sender_account, self.contract_address()),
            None,
            Options::default(),
            None,
        );
        let current_allowance: U256 = query
            .compat()
            .await
            .map_err(|err| ClientError::NetworkError(err.to_string()))?;

        Ok(current_allowance >= erc20_approve_threshold)
    }

    /// Sends a transaction to ERC20 token contract to approve the ERC20 deposit.
    pub async fn approve_erc20_token_deposits(
        &self,
        token: impl Into<TokenLike>,
    ) -> Result<H256, ClientError> {
        let token = token.into();
        let max_erc20_approve_amount: U256 =
            "115792089237316195423570985008687907853269984665640564039457584007913129639935"
                .parse()
                .unwrap(); // 2^256 - 1

        let token = self
            .tokens_cache
            .resolve(token)
            .ok_or(ClientError::UnknownToken)?;

        let contract_function = self
            .erc20_abi
            .function("approve")
            .expect("failed to get function parameters");
        let params = (self.contract_address(), max_erc20_approve_amount);
        let data = contract_function
            .encode_input(&params.into_tokens())
            .expect("failed to encode parameters");

        let signed_tx = self
            .eth_client
            .sign_prepared_tx_for_addr(data, token.address, Default::default())
            .await
            .map_err(|_| ClientError::IncorrectCredentials)?;

        let transactin_hash = self
            .eth_client
            .send_raw_tx(signed_tx.raw_tx)
            .await
            .map_err(|err| ClientError::NetworkError(err.to_string()))?;

        Ok(transactin_hash)
    }

    /// Performs a transfer of funds from one Ethereum account to another.
    /// Note: This operation is performed on Ethereum, and not related to zkSync directly.
    pub async fn transfer(
        &self,
        token: impl Into<TokenLike>,
        amount: U256,
        to: H160,
    ) -> Result<H256, ClientError> {
        let token = token.into();
        let token_info = self
            .tokens_cache
            .resolve(token.clone())
            .ok_or(ClientError::UnknownToken)?;

        let signed_tx = if self.tokens_cache.is_eth(token) {
            let mut options = Options::default();
            options.value = Some(amount);
            self.eth_client
                .sign_prepared_tx_for_addr(Vec::new(), to, options)
                .await
                .map_err(|_| ClientError::IncorrectCredentials)?
        } else {
            let contract_function = self
                .erc20_abi
                .function("transfer")
                .expect("failed to get function parameters");
            let params = (to, amount);
            let data = contract_function
                .encode_input(&params.into_tokens())
                .expect("failed to encode parameters");

            self.eth_client
                .sign_prepared_tx_for_addr(data, token_info.address, Default::default())
                .await
                .map_err(|_| ClientError::IncorrectCredentials)?
        };

        let transaction_hash = self
            .eth_client
            .send_raw_tx(signed_tx.raw_tx)
            .await
            .map_err(|err| ClientError::NetworkError(err.to_string()))?;

        Ok(transaction_hash)
    }

    /// Performs a deposit in zkSync network.
    /// For ERC20 tokens, a deposit must be approved beforehand via the `EthereumProvider::approve_erc20_token_deposits` method.
    pub async fn deposit(
        &self,
        token: impl Into<TokenLike>,
        amount: U256,
        sync_address: H160,
    ) -> Result<H256, ClientError> {
        let token = token.into();
        let token_info = self
            .tokens_cache
            .resolve(token.clone())
            .ok_or(ClientError::UnknownToken)?;

        let signed_tx = if self.tokens_cache.is_eth(token) {
            let mut options = Options::default();
            options.value = Some(amount);
            options.gas = Some(200_000.into());
            self.eth_client
                .sign_call_tx("depositETH", sync_address, options)
                .await
                .map_err(|_| ClientError::IncorrectCredentials)?
        } else {
            let mut options = Options::default();
            options.gas = Some(200_000.into());
            let params = (token_info.address, amount, sync_address);
            self.eth_client
                .sign_call_tx("depositETH", params, options)
                .await
                .map_err(|_| ClientError::IncorrectCredentials)?
        };

        let transaction_hash = self
            .eth_client
            .send_raw_tx(signed_tx.raw_tx)
            .await
            .map_err(|err| ClientError::NetworkError(err.to_string()))?;

        Ok(transaction_hash)
    }

    /// Performs a full exit for a certain token.
    pub async fn full_exit(
        &self,
        token: impl Into<TokenLike>,
        account_id: AccountId,
    ) -> Result<H256, ClientError> {
        let token = token.into();
        let token = self
            .tokens_cache
            .resolve(token.clone())
            .ok_or(ClientError::UnknownToken)?;
        let account_id = U256::from(account_id);

        let mut options = Options::default();
        options.gas = Some(500_000.into());

        let signed_tx = self
            .eth_client
            .sign_call_tx("fullExit", (account_id, token.address), options)
            .await
            .map_err(|_| ClientError::IncorrectCredentials)?;

        let transactin_hash = self
            .eth_client
            .send_raw_tx(signed_tx.raw_tx)
            .await
            .map_err(|err| ClientError::NetworkError(err.to_string()))?;

        Ok(transactin_hash)
    }
}
