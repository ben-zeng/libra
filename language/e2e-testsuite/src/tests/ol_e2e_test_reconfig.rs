// Copyright (c) 0lsf
// SPDX-License-Identifier: Apache-2.0

use language_e2e_tests::{
    account::{Account, AccountData, AccountRoleSpecifier},
    common_transactions::{create_validator_account_txn},
    executor::FakeExecutor,
    transaction_status_eq,
    reconfig_setup::{bulk_update, bulk_update_setup}
};
use libra_types::{
    account_config::{testnet_dd_account_address, from_currency_code_string, lbr_type_tag},
    transaction::TransactionStatus,
    vm_status::{StatusCode, VMStatus, KeptVMStatus},
};
use std::convert::TryInto;
use transaction_builder::*;
pub const LBR_NAME: &str = "GAS";

#[test]
fn reconfig_bulk_update_test () {
    // Run with: `cargo xtest -p language-e2e-testsuite reconfig_bulk_update_test -- --nocapture`
    let mut executor = FakeExecutor::from_genesis_file();
    let mut sequence_number = 1u64;

    // NOTE: While true that the VM will initialize with some validators, this 
    // test involving checking the size and members of the validator set in move.
    // So, even though there are some validators already created, this test is 
    // run with five new validators.

    // Create some account types to be able to call a tx script and be validators
    // let association_account = Account::new_association();
    let libra_root = Account::new_libra_root();
    //let dd = Account::new_genesis_account(testnet_dd_account_address());
    let mut accounts = vec![];
    for _i in 0..5 {
        accounts.push(Account::new());
    }
    println!("created new validator accounts");

    // Add the account datas
    let libra_root_data = AccountData::with_account(
        libra_root, 1_000_000_000_000,
        from_currency_code_string(LBR_NAME).unwrap(), sequence_number, AccountRoleSpecifier::LibraRoot);
    executor.add_account_data(&libra_root_data);

    let names = vec!["alice", "bob", "carol", "sha", "ram"];
    // Create a transaction allowing the accounts to serve as validators
    for i in 0..5 {
        //let txn = create_validator_account_txn(&libra_root, accounts.get(i).unwrap(), (i+1).try_into().unwrap(), (**names.get(i).unwrap()).to_string().into_bytes());
        //executor.execute_and_apply(txn);
        executor.execute_and_apply(
            libra_root_data.account().transaction()
                .script(encode_create_validator_account_script(
                    sequence_number,
                    *accounts.get(i).unwrap().address(),
                    accounts.get(i).unwrap().auth_key_prefix(),
                    (**names.get(i).unwrap()).to_string().into_bytes(),
                ))
                .sequence_number(sequence_number)
                .sign(),
        );
        sequence_number+=1;
    }
    println!("registered new validator accounts");
    executor.new_block();
    
    //Give the validators some money
    let mint_amount = 1_000_000;
    for i in 0..5 {
        // executor.execute_and_apply(assoc_acc_data.account().signed_script_txn(
        //     encode_mint_script(lbr_type_tag(), accounts.get(i).unwrap().address(), vec![], mint_amount),
        //     (i + 6).try_into().unwrap(),
        // ));

        //////////////////////////////////////////////////////////////////////////////////////////////
        // with libra root
        // error: 4615
        // executor.execute_and_apply(
        //     libra_root_data.account().transaction()
        //         .script(encode_peer_to_peer_with_metadata_script(
        //             lbr_type_tag(),
        //             *accounts.get(i).unwrap().address(),
        //             mint_amount,
        //             vec![],
        //             vec![],
        //         ))
        //         .sequence_number(sequence_number)
        //         .sign(),
        // );
        //sequence_number+=1;

        //////////////////////////////////////////////////////////////////////////////////////////////
        // with genesis account
        // see account_limits.rs for a working example, they use coin1 instead of libra
        // error: 1288: not enough balance, see peer_to_peer.rs:94
        // can't set the balance for genesis account as it doesn't fall into any types in AccountRoleSpecifier
        // 
        // executor.execute_and_apply(
        //     dd.transaction()
        //         .script(encode_peer_to_peer_with_metadata_script(
        //             lbr_type_tag(),
        //             *accounts.get(i).unwrap().address(),
        //             mint_amount,
        //             vec![],
        //             vec![],
        //         ))
        //         .sequence_number((i).try_into().unwrap())
        //         .sign(),
        // );
        
    }
    println!("minted tokens for validators");
    executor.new_block();

    //////////////////////////////////////////////////////////////////////////////////////////////
    // register validator config
    // passed
    for i in 0..5 {
        executor.execute_and_apply(
            accounts.get(i).unwrap()
                .transaction()
                .script(encode_register_validator_config_script(
                    *accounts.get(i).unwrap().address(),
                    [
                        0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9,
                        0x64, 0x07, 0x3a, 0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02,
                        0x1a, 0x68, 0xf7, 0x07, 0x51, 0x1a,
                    ]
                    .to_vec(),
                    vec![254; 32],
                    vec![253; 32],
                ))
                .sequence_number(0)
                .sign(),
        );
    }
    println!("registered validators");
    executor.new_block();

    //////////////////////////////////////////////////////////////////////////////////////////////
    // Actually register the accounts as validators
    // passed
    for i in 0..5 {
        executor.execute_and_apply(
            libra_root_data.account()
                .transaction()
                .script(encode_add_validator_and_reconfigure_script(
                    sequence_number,
                    (**names.get(i).unwrap()).to_string().into_bytes(),
                    *accounts.get(i).unwrap().address(),
                ))
                .sequence_number(sequence_number)
                .sign(),
        );
        sequence_number+=1;
    }
    println!("registered and reconfiged validators");
    executor.new_block();

    //////////////////////////////////////////////////////////////////////////////////////////////
    // Construct the signed tx script for test setup.
    // This removes default validators and adds ours instead.
    // EXECUTION_FAILURE { status_code: MISSING_DATA, location: 00000000000000000000000000000001::MinerState, function_definition: 4, 
        // code_offset: 1 }, output TransactionOutput { write_set: WriteSet(WriteSetMut { write_set: [(AccessPath { address: 
        // 00000000000000000000000000000000, path: 0185aa1f79e25ca70b8f54f8a0b673259fc19b1af35acb98885d99e160600387c9 }, 
        // ...
    let setup = bulk_update_setup(&libra_root_data.account(), &accounts, sequence_number);

    // Execute and persist the txn in a new block
    executor.new_block();
    let tx_out = executor.execute_and_apply(setup);

    // Assert success
    assert_eq!(tx_out.status().status(), Ok(KeptVMStatus::Executed));
}
