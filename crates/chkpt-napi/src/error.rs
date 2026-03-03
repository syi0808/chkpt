use chkpt_core::error::ChkpttError;

pub fn to_napi_error(err: ChkpttError) -> napi::Error {
    napi::Error::new(napi::Status::GenericFailure, err.to_string())
}
