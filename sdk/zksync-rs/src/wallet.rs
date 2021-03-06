use models::node::{AccountId, Address};

use crate::{
    credentials::WalletCredentials, error::ClientError, ethereum::EthereumProvider, operations::*,
    provider::Provider, signer::Signer, tokens_cache::TokensCache,
};

#[derive(Debug)]
pub struct Wallet {
    pub provider: Provider,
    pub signer: Signer,
    pub tokens: TokensCache,
}

impl Wallet {
    pub async fn new(
        provider: Provider,
        credentials: WalletCredentials,
    ) -> Result<Self, ClientError> {
        let mut signer = Signer::new(
            credentials.zksync_private_key,
            credentials.eth_address,
            credentials.eth_private_key,
        );

        let account_info = provider.account_info(credentials.eth_address).await?;
        signer.set_account_id(account_info.id);

        let tokens = TokensCache::new(provider.tokens().await?);

        Ok(Wallet {
            provider,
            signer,
            tokens,
        })
    }

    /// Updates account ID stored in the wallet.
    /// This method must be invoked if the wallet was created for a non-existent account,
    /// and it was initialized after creation (e.g. by doing a deposit).
    /// If zkSync wallet was initialized, but information in `Wallet` object was not updated,
    /// `Wallet` won't be able to perform any zkSync transaction.
    pub async fn update_account_id(&mut self) -> Result<(), ClientError> {
        let account_info = self.provider.account_info(self.address()).await?;
        self.signer.set_account_id(account_info.id);
        Ok(())
    }

    /// Returns the wallet address.
    pub fn address(&self) -> Address {
        self.signer.address
    }

    /// Returns the current account ID.
    /// Result may be `None` if the signing key was not set for account via `ChangePubKey` transaction.
    pub fn account_id(&self) -> Option<AccountId> {
        self.signer.account_id
    }

    /// Updates the list of tokens supported by zkSync.
    /// This method only needs to be called if a new token was added to zkSync after
    /// `Wallet` object was created.
    pub async fn refresh_tokens_cache(&mut self) -> Result<(), ClientError> {
        self.tokens = TokensCache::new(self.provider.tokens().await?);

        Ok(())
    }

    /// Returns `true` if signing key for account was set in zkSync network.
    /// In other words, returns `true` if `ChangePubKey` operation was performed for the
    /// account.
    ///
    /// If this method has returned `false`, one must send a `ChangePubKey` transaction
    /// via `Wallet::start_change_pubkey` method.
    pub async fn is_signing_key_set(&self) -> Result<bool, ClientError> {
        let account_info = self.provider.account_info(self.address()).await?;

        let key_set =
            account_info.id.is_some() && account_info.committed.pub_key_hash != Default::default();
        Ok(key_set)
    }

    /// Initializes `Transfer` transaction sending.
    pub fn start_transfer(&self) -> TransferBuilder<'_> {
        TransferBuilder::new(self)
    }

    /// Initializes `ChangePubKey` transaction sending.
    pub fn start_change_pubkey(&self) -> ChangePubKeyBuilder<'_> {
        ChangePubKeyBuilder::new(self)
    }

    /// Initializes `Withdraw` transaction sending.
    pub fn start_withdraw(&self) -> WithdrawBuilder<'_> {
        WithdrawBuilder::new(self)
    }

    /// Creates an `EthereumProvider` to interact with the Ethereum network.
    ///
    /// Returns an error if wallet was created without providing an Ethereum private key.
    pub async fn ethereum(
        &self,
        web3_addr: impl AsRef<str>,
    ) -> Result<EthereumProvider, ClientError> {
        if let Some(eth_private_key) = self.signer.eth_private_key {
            let ethereum_provider = EthereumProvider::new(
                &self.provider,
                self.tokens.clone(),
                web3_addr,
                eth_private_key,
                self.signer.address,
            )
            .await?;

            Ok(ethereum_provider)
        } else {
            Err(ClientError::NoEthereumPrivateKey)
        }
    }
}
