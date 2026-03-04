pub fn dependency_contract_snapshot() -> (
    anyhow::Result<()>,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
) {
    (
        Ok(()),
        std::any::type_name::<hmac::Hmac<sha2::Sha256>>(),
        std::any::type_name::<inquire::ui::RenderConfig>(),
        std::any::type_name::<lexopt::Parser>(),
        std::any::type_name::<sha2::Sha256>(),
        std::any::type_name::<tokio::runtime::Runtime>(),
        std::any::type_name::<turso::Builder>(),
    )
}

#[cfg(test)]
mod tests {
    use super::dependency_contract_snapshot;

    #[test]
    fn dependency_contract_snapshot_references_agreed_crates() {
        let (result, hmac_ty, inquire_ty, lexopt_ty, sha2_ty, tokio_ty, turso_ty) =
            dependency_contract_snapshot();
        assert!(result.is_ok());
        assert!(hmac_ty.contains("hmac::"));
        assert!(inquire_ty.contains("inquire::"));
        assert!(lexopt_ty.contains("lexopt::"));
        assert!(sha2_ty.contains("sha2::"));
        assert!(tokio_ty.contains("tokio::"));
        assert!(turso_ty.contains("turso::"));
    }
}
