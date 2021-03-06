// Built-in deps
// External imports
// Workspace imports
use models::{node::BlockNumber, ActionType};
// Local imports
use self::records::{
    NewExecutedPriorityOperation, NewExecutedTransaction, NewOperation,
    StoredExecutedPriorityOperation, StoredExecutedTransaction, StoredOperation,
};
use crate::{chain::mempool::MempoolSchema, QueryResult, StorageProcessor};

pub mod records;

/// Operations schema is capable of storing and loading the transactions.
/// Every kind of transaction (non-executed, executed, and executed priority tx)
/// can be either saved or loaded from the database.
#[derive(Debug)]
pub struct OperationsSchema<'a, 'c>(pub &'a mut StorageProcessor<'c>);

impl<'a, 'c> OperationsSchema<'a, 'c> {
    pub async fn get_last_block_by_action(
        &mut self,
        action_type: ActionType,
    ) -> QueryResult<BlockNumber> {
        let max_block = sqlx::query!(
            r#"SELECT max(block_number) FROM operations WHERE action_type = $1"#,
            action_type.to_string()
        )
        .fetch_one(self.0.conn())
        .await?
        .max
        .unwrap_or(0);

        Ok(max_block as BlockNumber)
    }

    pub async fn get_operation(
        &mut self,
        block_number: BlockNumber,
        action_type: ActionType,
    ) -> Option<StoredOperation> {
        sqlx::query_as!(
            StoredOperation,
            "SELECT * FROM operations WHERE block_number = $1 AND action_type = $2",
            i64::from(block_number),
            action_type.to_string()
        )
        .fetch_optional(self.0.conn())
        .await
        .ok()
        .flatten()
    }

    pub async fn get_executed_operation(
        &mut self,
        op_hash: &[u8],
    ) -> QueryResult<Option<StoredExecutedTransaction>> {
        let op = sqlx::query_as!(
            StoredExecutedTransaction,
            "SELECT * FROM executed_transactions WHERE tx_hash = $1",
            op_hash
        )
        .fetch_optional(self.0.conn())
        .await?;

        Ok(op)
    }

    pub async fn get_executed_priority_operation(
        &mut self,
        priority_op_id: u32,
    ) -> QueryResult<Option<StoredExecutedPriorityOperation>> {
        let op = sqlx::query_as!(
            StoredExecutedPriorityOperation,
            "SELECT * FROM executed_priority_operations WHERE priority_op_serialid = $1",
            i64::from(priority_op_id)
        )
        .fetch_optional(self.0.conn())
        .await?;

        Ok(op)
    }

    pub async fn get_executed_priority_operation_by_hash(
        &mut self,
        eth_hash: &[u8],
    ) -> QueryResult<Option<StoredExecutedPriorityOperation>> {
        let op = sqlx::query_as!(
            StoredExecutedPriorityOperation,
            "SELECT * FROM executed_priority_operations WHERE eth_hash = $1",
            eth_hash
        )
        .fetch_optional(self.0.conn())
        .await?;

        Ok(op)
    }

    pub(crate) async fn store_operation(
        &mut self,
        operation: NewOperation,
    ) -> QueryResult<StoredOperation> {
        let op = sqlx::query_as!(
            StoredOperation,
            "INSERT INTO operations (block_number, action_type) VALUES ($1, $2)
            RETURNING *",
            operation.block_number,
            operation.action_type
        )
        .fetch_one(self.0.conn())
        .await?;
        Ok(op)
    }

    /// Stores the executed operation in the database.
    pub(crate) async fn store_executed_operation(
        &mut self,
        operation: NewExecutedTransaction,
    ) -> QueryResult<()> {
        let mut transaction = self.0.start_transaction().await?;

        MempoolSchema(&mut transaction)
            .remove_tx(&operation.tx_hash)
            .await?;

        if operation.success {
            // If transaction succeed, it should replace the stored tx with the same hash.
            // The situation when a duplicate tx is stored in the database may exist only if has
            // failed previously.
            // Possible scenario: user had no enough funds for transfer, then deposited some and
            // sent the same transfer again.

            sqlx::query!(
                "INSERT INTO executed_transactions (block_number, block_index, tx, operation, tx_hash, from_account, to_account, success, fail_reason, primary_account_address, nonce, created_at, eth_sign_data)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                ON CONFLICT (tx_hash)
                DO UPDATE
                SET block_number = $1, block_index = $2, tx = $3, operation = $4, tx_hash = $5, from_account = $6, to_account = $7, success = $8, fail_reason = $9, primary_account_address = $10, nonce = $11, created_at = $12, eth_sign_data = $13",
                operation.block_number,
                operation.block_index,
                operation.tx,
                operation.operation,
                operation.tx_hash,
                operation.from_account,
                operation.to_account,
                operation.success,
                operation.fail_reason,
                operation.primary_account_address,
                operation.nonce,
                operation.created_at,
                operation.eth_sign_data,
            )
            .execute(transaction.conn())
            .await?;
        } else {
            // If transaction failed, we do nothing on conflict.
            sqlx::query!(
                "INSERT INTO executed_transactions (block_number, block_index, tx, operation, tx_hash, from_account, to_account, success, fail_reason, primary_account_address, nonce, created_at, eth_sign_data)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                ON CONFLICT (tx_hash)
                DO NOTHING",
                operation.block_number,
                operation.block_index,
                operation.tx,
                operation.operation,
                operation.tx_hash,
                operation.from_account,
                operation.to_account,
                operation.success,
                operation.fail_reason,
                operation.primary_account_address,
                operation.nonce,
                operation.created_at,
                operation.eth_sign_data,
            )
            .execute(transaction.conn())
            .await?;
        };

        transaction.commit().await?;
        Ok(())
    }

    pub(crate) async fn store_executed_priority_operation(
        &mut self,
        operation: NewExecutedPriorityOperation,
    ) -> QueryResult<()> {
        sqlx::query!(
            "INSERT INTO executed_priority_operations (block_number, block_index, operation, from_account, to_account, priority_op_serialid, deadline_block, eth_hash, eth_block, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (priority_op_serialid)
            DO NOTHING",
            operation.block_number,
            operation.block_index,
            operation.operation,
            operation.from_account,
            operation.to_account,
            operation.priority_op_serialid,
            operation.deadline_block,
            operation.eth_hash,
            operation.eth_block,
            operation.created_at,
        )
        .execute(self.0.conn())
        .await?;
        Ok(())
    }
}
