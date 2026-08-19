use crate::HashaiError;

pub fn validate(os: &str, shell: &str) -> Result<(), HashaiError> {
    if !matches!(os, "linux" | "macos") {
        return Err(HashaiError::UnsupportedPlatform(format!(
            "unsupported operating system `{os}`; hashai supports Linux and macOS"
        )));
    }
    crate::config::Shell::parse(shell)?;
    Ok(())
}
