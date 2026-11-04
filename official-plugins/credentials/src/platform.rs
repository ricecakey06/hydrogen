use hydrogen_plugin_sdk::contracts::{ApiError, Result};
pub fn credentials(value: &mut [u8], protect: bool) -> Result<Vec<u8>> {
    #[cfg(windows)]
    unsafe {
        use windows::{
            Win32::{
                Foundation::{HLOCAL, LocalFree},
                Security::Cryptography::{
                    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData,
                    CryptUnprotectData,
                },
            },
            core::PCWSTR,
        };
        let input = CRYPT_INTEGER_BLOB {
            cbData: value.len() as u32,
            pbData: value.as_mut_ptr(),
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        let result = if protect {
            CryptProtectData(
                &input,
                PCWSTR::null(),
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        value.fill(0);
        result.map_err(|_| {
            ApiError::new(
                "CREDENTIAL_PROTECTION",
                "当前 Windows 用户无法加密或解密凭据。",
            )
        })?;
        let result = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize).fill(0);
        let _ = LocalFree(Some(HLOCAL(output.pbData as _)));
        Ok(result)
    }
    #[cfg(not(windows))]
    {
        let _ = protect;
        value.fill(0);
        Err(ApiError::new(
            "SESSION_ONLY",
            "当前平台仅支持本次会话登录。",
        ))
    }
}
