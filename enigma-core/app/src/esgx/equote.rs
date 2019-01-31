use common_u::errors;
use failure::Error;
use sgx_types::*;
use std::str;

extern "C" {
    fn ecall_get_signing_address(eid: sgx_enclave_id_t, pubkey: &mut [u8; 42]) -> sgx_status_t;
}

// this struct is returned during the process registration back to the surface.
// quote: the base64 encoded quote
// address : the clear text public key for ecdsa signing and registration
#[derive(Serialize, Deserialize, Debug)]
pub struct GetRegisterResult {
    pub errored: bool,
    pub quote: String,
    pub address: String,
}

// wrapper function for getting the enclave public sign key (the one attached with produce_quote())
#[logfn(DEBUG)]
pub fn get_register_signing_address(eid: sgx_enclave_id_t) -> Result<String, Error> {
    let mut address: [u8; 42] = [0; 42];
    let status = unsafe { ecall_get_signing_address(eid, &mut address) };
    if status == sgx_status_t::SGX_SUCCESS {
        let address_str = str::from_utf8(&address).unwrap();
        Ok(address_str.to_owned())
    } else {
        Err(errors::GetRegisterKeyErr { status, message: String::from("error in get_register_signing_key") }.into())
    }
}

#[cfg(test)]
mod test {
    use crate::esgx::general::init_enclave_wrapper;
    use enigma_tools_u::attestation_service::{self, service::AttestationService};
    use enigma_tools_u::esgx::equote::retry_quote;

    // isans SPID = "3DDB338BD52EE314B01F1E4E1E84E8AA"
    // victors spid = 68A8730E9ABF1829EA3F7A66321E84D0
    const SPID: &str = "1601F95C39B9EA307FEAABB901ADC3EE"; // Elichai's SPID

    #[test]
    fn test_produce_quote() {
        // initiate the enclave
        let enclave = init_enclave_wrapper().unwrap();
        // produce a quote

        let tested_encoded_quote = match retry_quote(enclave.geteid(), &SPID, 18) {
            Ok(encoded_quote) => encoded_quote,
            Err(e) => {
                println!("[-] Produce quote Err {}, {}", e.as_fail(), e.backtrace());
                assert_eq!(0, 1);
                return;
            }
        };
        println!("-------------------------");
        println!("{}", tested_encoded_quote);
        println!("-------------------------");
        enclave.destroy();
        assert!(tested_encoded_quote.len() > 0);
        //assert_eq!(real_encoded_quote, tested_encoded_quote);
    }

    #[test]
    fn test_produce_and_verify_qoute() {
        let enclave = init_enclave_wrapper().unwrap();
        let quote = retry_quote(enclave.geteid(), &SPID, 18).unwrap();
        let service = AttestationService::new(attestation_service::constants::ATTESTATION_SERVICE_URL);
        let as_response = service.get_report(&quote).unwrap();

        assert!(as_response.result.verify_report().unwrap());
    }
}
