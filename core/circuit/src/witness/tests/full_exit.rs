// External deps
use bigdecimal::BigDecimal;
// Workspace deps
use models::node::{operations::FullExitOp, Account, FullExit};
use testkit::zksync_account::ZksyncAccount;
// Local deps
use crate::witness::{
    full_exit::{apply_full_exit_tx, calculate_full_exit_operations_from_witness},
    tests::test_utils::{check_circuit, test_genesis_plasma_state},
    utils::WitnessBuilder,
};

#[test]
#[ignore]
fn test_full_exit_success() {
    let zksync_account = ZksyncAccount::rand();
    let account_id = 1;
    let account_address = zksync_account.address;
    let account = {
        let mut account = Account::default_with_address(&account_address);
        account.add_balance(0, &BigDecimal::from(10));
        account.pub_key_hash = zksync_account.pubkey_hash;
        account
    };

    let (mut plasma_state, mut circuit_account_tree) =
        test_genesis_plasma_state(vec![(account_id, account)]);
    let fee_account_id = 0;
    let mut witness_accum = WitnessBuilder::new(&mut circuit_account_tree, fee_account_id, 1);

    let full_exit_op = FullExitOp {
        priority_op: FullExit {
            account_id,
            eth_address: account_address,
            token: 0,
        },
        withdraw_amount: Some(BigDecimal::from(10)),
    };

    println!("node root hash before op: {:?}", plasma_state.root_hash());
    plasma_state.apply_full_exit_op(&full_exit_op);
    println!("node root hash after op: {:?}", plasma_state.root_hash());

    let full_exit_witness =
        apply_full_exit_tx(&mut witness_accum.account_tree, &full_exit_op, true);
    let full_exit_operations = calculate_full_exit_operations_from_witness(&full_exit_witness);
    let pubdata_from_witness = full_exit_witness.get_pubdata();

    witness_accum.add_operation_with_pubdata(full_exit_operations, pubdata_from_witness);
    witness_accum.collect_fees(&[]);
    witness_accum.calculate_pubdata_commitment();

    assert_eq!(
        plasma_state.root_hash(),
        witness_accum
            .root_after_fees
            .expect("witness accum after root hash empty"),
        "root hash in state keeper and witness generation code mismatch"
    );

    check_circuit(witness_accum.into_circuit_instance());
}

#[test]
#[ignore]
fn test_full_exit_failure_no_account_in_tree() {
    let zksync_account = ZksyncAccount::rand();
    let account_id = 1;
    let account_address = zksync_account.address;

    let (mut plasma_state, mut circuit_account_tree) = test_genesis_plasma_state(Vec::new());
    let fee_account_id = 0;
    let mut witness_accum = WitnessBuilder::new(&mut circuit_account_tree, fee_account_id, 1);

    let full_exit_op = FullExitOp {
        priority_op: FullExit {
            account_id,
            eth_address: account_address,
            token: 0,
        },
        withdraw_amount: None,
    };

    println!("node root hash before op: {:?}", plasma_state.root_hash());
    plasma_state.apply_full_exit_op(&full_exit_op);
    println!("node root hash after op: {:?}", plasma_state.root_hash());

    let full_exit_witness =
        apply_full_exit_tx(&mut witness_accum.account_tree, &full_exit_op, false);
    let full_exit_operations = calculate_full_exit_operations_from_witness(&full_exit_witness);
    let pubdata_from_witness = full_exit_witness.get_pubdata();

    witness_accum.add_operation_with_pubdata(full_exit_operations, pubdata_from_witness);
    witness_accum.collect_fees(&[]);
    witness_accum.calculate_pubdata_commitment();

    assert_eq!(
        plasma_state.root_hash(),
        witness_accum
            .root_after_fees
            .expect("witness accum after root hash empty"),
        "root hash in state keeper and witness generation code mismatch"
    );

    check_circuit(witness_accum.into_circuit_instance());
}
