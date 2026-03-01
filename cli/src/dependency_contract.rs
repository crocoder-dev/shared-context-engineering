pub fn dependency_contract_snapshot(
) -> (anyhow::Result<()>, &'static str, &'static str, &'static str) {
    (
        Ok(()),
        std::any::type_name::<lexopt::Parser>(),
        std::any::type_name::<tokio::runtime::Runtime>(),
        std::any::type_name::<turso::Builder>(),
    )
}

#[cfg(test)]
mod tests {
    use super::dependency_contract_snapshot;

    #[test]
    fn dependency_contract_snapshot_references_agreed_crates() {
        let (result, lexopt_ty, tokio_ty, turso_ty) = dependency_contract_snapshot();
        assert!(result.is_ok());
        assert!(lexopt_ty.contains("lexopt::"));
        assert!(tokio_ty.contains("tokio::"));
        assert!(turso_ty.contains("turso::"));
    }
}
