//! Current kernel lane declarations; composition supplies their execution hosts.

pub mod artifacts;
pub mod audit;
pub mod gateway;
pub mod identity;
pub mod knowledge;
pub mod store;
pub mod tasks;

/// The complete always-selected kernel catalog, with execution supplied by composition.
pub fn kernel_modules(
    mut execution: impl FnMut(
        &str,
    )
        -> Result<veoveo_modules::LaneExecution, veoveo_modules::DeclarationError>,
) -> Result<Vec<veoveo_modules::ModuleSetup>, veoveo_modules::DeclarationError> {
    Ok(vec![
        store::module_setup(execution("store")?)?,
        identity::module_setup(execution("identity")?)?,
        gateway::module_setup(execution("gateway")?)?,
        artifacts::module_setup(execution("artifacts")?)?,
        tasks::module_setup(execution("tasks")?)?,
        audit::module_setup(execution("audit")?)?,
        knowledge::module_setup(execution("knowledge")?)?,
    ])
}

#[cfg(test)]
mod reserved_names {
    #[test]
    fn kernel_module_names_are_reserved_slugs() {
        let execution = |name: &str| {
            veoveo_modules::LaneExecution::new(
                veoveo_modules::ExecutionImage::new("gateway")?,
                veoveo_modules::ExecutionCommand::new(vec![
                    "migrate".into(),
                    "--module".into(),
                    name.into(),
                ])?,
            )
        };
        for module in super::kernel_modules(execution).unwrap() {
            assert!(
                veoveo_types::RESERVED_SERVER_SLUGS.contains(&module.name().as_str()),
                "{} must be a reserved slug",
                module.name().as_str()
            );
        }
    }
}
