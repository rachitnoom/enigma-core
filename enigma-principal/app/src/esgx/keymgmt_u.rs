use failure::Error;
// general
//sgx
use sgx_types::{sgx_enclave_id_t, sgx_status_t};
use web3::types::{BlockHeader, H256, Log};
use enigma_tools_u::web3_utils::provider_types::{encode, BlockHeaderWrapper, ReceiptWrapper, ReceiptHashesWrapper, BlockHeaders};
use boot_network::principal_server::{StateKeyRequest, StateKeyResponse, StringWrapper};
use common_u::errors;

extern {
    fn ecall_get_random_seed(eid: sgx_enclave_id_t, retval: &mut sgx_status_t,
                             rand_out: &mut [u8; 32], sig_out: &mut [u8; 65]) -> sgx_status_t;

    fn ecall_set_worker_params(eid: sgx_enclave_id_t, retval: &mut sgx_status_t,
                               receipt_rlp: *const u8, receipt_rlp_len: usize,
                               receipt_hashes_rlp: *const u8, receipt_hashes_rlp_len: usize,
                               headers_rlp: *const u8, headers_rlp_len: usize,
                               sig_out: &mut [u8; 65]) -> sgx_status_t;

    fn ecall_get_enc_state_keys(eid: sgx_enclave_id_t, retval: &mut sgx_status_t,
                                enc_msg: *const u8, enc_msg_len: usize,
                                sig: &[u8; 65],
                                enc_response_out: *mut u8, enc_response_len_out: &mut usize,
                                sig_out: &mut [u8; 65]) -> sgx_status_t;
}

pub fn generate_epoch_seed(eid: sgx_enclave_id_t) -> ([u8; 32], [u8; 65]) {
    let mut rand_out: [u8; 32] = [0; 32];
    let mut sig_out: [u8; 65] = [0; 65];
    let mut retval = sgx_status_t::default();
    let result = unsafe {
        ecall_get_random_seed(eid, &mut retval, &mut rand_out, &mut sig_out)
    };
    assert_eq!(result, sgx_status_t::SGX_SUCCESS); // TODO: Replace with good Error handling.
    (rand_out, sig_out)
}

pub fn set_worker_params(eid: sgx_enclave_id_t, receipt: ReceiptWrapper, receipt_hashes: ReceiptHashesWrapper, headers: BlockHeaders) -> Result<[u8; 65], Error> {
    // Serialize the receipt into RLP
    let receipt_rlp = encode(&receipt);
    let receipt_hashes_rlp = encode(&receipt_hashes);
    let headers_rlp = encode(&headers);
    let mut sig_out: [u8; 65] = [0; 65];
    let mut retval = sgx_status_t::default();
    let result = unsafe {
        ecall_set_worker_params(
            eid,
            &mut retval,
            receipt_rlp.as_ptr() as *const u8,
            receipt_rlp.len(),
            receipt_hashes_rlp.as_ptr() as *const u8,
            receipt_hashes_rlp.len(),
            headers_rlp.as_ptr() as *const u8,
            headers_rlp.len(),
            &mut sig_out,
        )
    };
    if result.ne(&sgx_status_t::SGX_SUCCESS) {
        return Err(errors::WorkerParamsErr { message: "See Enclave logs for details.".to_string() }.into());
    }
    Ok(sig_out)
}

const MAX_ENC_RESPONSE_LEN: usize = 100_000;

pub fn get_enc_state_keys(eid: sgx_enclave_id_t, request: StateKeyRequest) -> Result<StateKeyResponse, Error> {
    let mut sig_out: [u8; 65] = [0; 65];
    let mut enc_response = vec![0u8; MAX_ENC_RESPONSE_LEN];
    let enc_response_slice = enc_response.as_mut_slice();
    let mut enc_response_len_out: usize = 0;
    let mut retval = sgx_status_t::default();

    let enc_msg: Vec<u8> = request.request_message.into();
    let response = unsafe {
        ecall_get_enc_state_keys(
            eid,
            &mut retval,
            enc_msg.as_ptr() as *const u8,
            enc_msg.len(),
            &request.worker_sig.into(),
            enc_response_slice.as_mut_ptr() as *mut u8,
            &mut enc_response_len_out,
            &mut sig_out,
        )
    };
    assert_eq!(response, sgx_status_t::SGX_SUCCESS); // TODO: Replace with good Error handling.
    println!("got encrypted state keys: {:?}", response);
    let enc_response_out: Vec<u8> = enc_response_slice[0..enc_response_len_out].iter().cloned().collect();
    Ok(StateKeyResponse {
        encrypted_response_message: StringWrapper::from(enc_response_out),
        sig: StringWrapper::from(sig_out),
    })
}

#[cfg(test)]
pub mod tests {
    #![allow(dead_code, unused_assignments, unused_variables)]

    use std::prelude::v1::Vec;

    use ethabi::{Address, Uint, Event, EventParam, ParamType};
    use rustc_hex::FromHex;
    use rustc_hex::ToHex;
    use serde_json as ser;
    use sgx_urts::SgxEnclave;
    use tiny_keccak::Keccak;
    use web3::types::{Bytes, TransactionReceipt, Block};
    use enigma_tools_u::web3_utils::provider_types::{BlockHeaderWrapper};
    use enigma_tools_u::web3_utils::keeper_types_u::{EventWrapper};

    use esgx;
    use esgx::random_u::get_signed_random;

    use super::*;

    fn init_enclave() -> SgxEnclave {
        let enclave = match esgx::general::init_enclave_wrapper() {
            Ok(r) => {
                println!("[+] Init Enclave Successful {}!", r.geteid());
                r
            }
            Err(x) => {
                panic!("[-] Init Enclave Failed {}!", x.as_str());
            }
        };
        enclave
    }

    //noinspection RsTypeCheck
    pub(crate) fn set_mock_worker_params(eid: sgx_enclave_id_t) -> ([u8; 65]) {
        const EXAMPLE_RECEIPT: &'static str = r#"{"transactionHash":"0x33c3c14e3cd8764911d243e67c229adf7279b3e920a3dbb317ff989946ad47bb","transactionIndex":0,"blockHash":"0x0c296afc063c64e6d439e68aee7e21711acae300b8c463b72a9eaa2fa6f01953","blockNumber":108,"from":"0x9dc9f269cea8b616df485d3da206df08175493fa","to":"0xefa854bef1878fef38c9fa9eda734bca4461703b","gasUsed":818482,"cumulativeGasUsed":818482,"contractAddress":null,"logs":[{"logIndex":0,"transactionIndex":0,"transactionHash":"0x33c3c14e3cd8764911d243e67c229adf7279b3e920a3dbb317ff989946ad47bb","blockHash":"0x0c296afc063c64e6d439e68aee7e21711acae300b8c463b72a9eaa2fa6f01953","blockNumber":108,"address":"0xeFA854beF1878feF38c9fa9EDa734BcA4461703B","data":"0x000000000000000000000000000000000000000000000000000000000000b084000000000000000000000000000000000000000000000000000000000000006c00000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000001c000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000008000000000000000000000000b4e4d9c2234d712d9bf6c9445f1cc402106bb02b0000000000000000000000005fa80853fab1c9b171968b7ea26172c981d870e3000000000000000000000000669b6a7969da392875bb58e17a83f712adac476e000000000000000000000000e8d52b919b9cf20e96b378166d94a7aac7cd0423000000000000000000000000bec594fffd4546544309f67eb4121606c5cfca3d000000000000000000000000761751ad640d7b9741a93acd755f6ba83be88e8e000000000000000000000000b25f28b6e9b316ef95060a47ed5ee80990fcc7840000000000000000000000002c4d2745716d18a3f4e8f7592ace4ba6bcf2448d000000000000000000000000000000000000000000000000000000000000000800000000000000000000000000000000000000000000000000000014f46b040000000000000000000000000000000000000000000000000000000002540be400000000000000000000000000000000000000000000000000000000003b9aca00000000000000000000000000000000000000000000000000000000007735940000000000000000000000000000000000000000000000000000000002540be40000000000000000000000000000000000000000000000000000000004a817c80000000000000000000000000000000000000000000000000000000000ee6b280000000000000000000000000000000000000000000000000000000002540be400","topics":["0xcdfdbdd264b9f454c9c98dc0d0b5c0a0f683c704db2233ff53b4d4f826c790cb"],"type":"mined","id":"log_bc15b82f"}],"status":true,"logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000040000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000008000000000000000","v":"0x1c","r":"0xb43597804dd468ed6a941c204ca34908e7ab1c82899534114aa80fc73c32f5ba","s":"0x1df8fbbf2f0c393f3111b243cfca6b0b44d0cf0cf6a6ebb8908a1890bba78cf8"}"#;
        const EXAMPLE_BLOCK: &'static str = r#"{"number":108,"hash":"0x0c296afc063c64e6d439e68aee7e21711acae300b8c463b72a9eaa2fa6f01953","parentHash":"0x88e10ea2b9be2e3285e96ce427d4fb6de93ab334279d5ded8c71302dcbc6b6aa","mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000","nonce":"0x0000000000000000","sha3Uncles":"0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347","logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000040000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000008000000000000000","transactionsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421","stateRoot":"0x59e01b2b2508581d30d755200675f35fc53d1a73e9fc0d9563c63b49adc0b8aa","receiptsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421","miner":"0x0000000000000000000000000000000000000000","difficulty":"0","totalDifficulty":"0","extraData":"0x","size":1000,"gasLimit":6721975,"gasUsed":818482,"timestamp":1546637172,"transactions":["0x33c3c14e3cd8764911d243e67c229adf7279b3e920a3dbb317ff989946ad47bb"],"uncles":[]}"#;

        let event = EventWrapper::workers_parameterized();
        let receipt = serde_json::from_str::<TransactionReceipt>(EXAMPLE_RECEIPT).unwrap();
        let block = serde_json::from_str::<Block<H256>>(EXAMPLE_RECEIPT).unwrap();
        let receipt = ReceiptWrapper { receipt:receipt.clone(), block:block.clone() };

        let mut hash: [u8; 32] = [0; 32];
        let mut sha3 = Keccak::new_sha3_256();
        sha3.update(&encode(receipt));
        sha3.finalize(&mut hash);
        let receipt_hashes = ReceiptHashesWrapper(vec![H256(hash)]);

        let headers = BlockHeaders(vec![BlockHeaderWrapper(block)]);
        return set_worker_params(eid, receipt, receipt_hashes, headers).unwrap();
    }

    #[test]
    fn test_set_worker_params() {
        let enclave = init_enclave();
        generate_epoch_seed(enclave.geteid());
        generate_epoch_seed(enclave.geteid());
        let sig = set_mock_worker_params(enclave.geteid());
        println!("got the data signature");
        enclave.destroy();
    }

    #[test]
    fn test_get_state_key() {
        let enclave = init_enclave();
        generate_epoch_seed(enclave.geteid());
        generate_epoch_seed(enclave.geteid());
        set_mock_worker_params(enclave.geteid());

        // From the km_primitives uint tests
        let msg = StringWrapper("84a46461746181a75265717565737493dc0020cca7cc937b64ccb8cccacca5cc8f03721bccb6ccbacccf5c78cccb235fccebcce0cce70b1bcc84cccdcc99541461cca0cc8edc002016367accacccb67a4a017ccc8dcca8ccabcc95682ccccb390863780f7114ccddcca0cca0cce0ccc55644ccc7ccc4dc0020ccb1cce9cc9324505bccd32dcca0cce1ccf85dcccf5e19cca0cc9dccb0481ecc8a15ccf62c41cceb320304cca8cce927a269649c1363ccb3301c101f33cce1cc9a0524a67072656669789e456e69676d61204d657373616765a67075626b6579dc0040cce5ccbe28cc9dcc9a2eccbd08ccc0457a5f16ccdfcc9fccdc256c5d5f6c3514cccdcc95ccb47c11ccc4cccd3e31ccf0cce4ccefccc83ccc80cce8121c3939ccbb2561cc80ccec48ccbecca8ccc569ccd2cca3ccda6bcce415ccfa20cc9bcc98ccda".to_string());
        let sig = StringWrapper("43f19586b0a0ae626b9418fe8355888013be1c9b4263a4b3a27953de641991e936ed6c4076a2a383b3b001936bf0eb6e23c78fbec1ee36f19c6a9d24d75e9e081c".to_string());
        println!("The mock message: {:?}", msg);
        println!("The mock sig: {:?}", sig);

        let request = StateKeyRequest { request_message: msg, worker_sig: sig };
        let response = get_enc_state_keys(enclave.geteid(), request).unwrap();
//        println!("Got response: {:?}", response);
        enclave.destroy();
    }
}
