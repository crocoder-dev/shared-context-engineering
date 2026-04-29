use std::collections::HashMap;

use crate::app::AppContext;
use crate::services::error::ClassifiedError;

/// Trait for executable CLI commands.
///
/// Each command implements this trait to provide a name and an execution
/// method that receives shared application context and returns a result
/// payload or a classified error.
pub trait RuntimeCommand {
    fn name(&self) -> std::borrow::Cow<'_, str>;

    fn execute(&self, context: &AppContext) -> Result<String, ClassifiedError>;
}

/// Owned handle to a dynamically dispatched runtime command.
pub type RuntimeCommandHandle = Box<dyn RuntimeCommand>;

/// Type alias for a command constructor that produces an owned command handle.
type CommandConstructor = fn() -> RuntimeCommandHandle;

/// Statically populated registry that maps command names to their constructors.
///
/// The registry is populated at compile time via [`build_default_registry`]
/// and looked up by name during command dispatch. Constructors are zero-arg
/// functions so the registry does not carry per-invocation state; per-invocation
/// data (e.g. parsed flags) is resolved by the parse layer before the command
/// is constructed.
pub struct CommandRegistry {
    constructors: HashMap<&'static str, CommandConstructor>,
}

impl CommandRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            constructors: HashMap::new(),
        }
    }

    /// Register a command constructor under the given name.
    ///
    /// If a constructor is already registered under `name`, it is replaced.
    #[allow(dead_code)]
    pub fn register(&mut self, name: &'static str, constructor: CommandConstructor) {
        self.constructors.insert(name, constructor);
    }

    /// Look up a command constructor by name.
    ///
    /// Returns `None` if no constructor is registered under `name`.
    #[allow(dead_code)]
    pub fn get(&self, name: &str) -> Option<CommandConstructor> {
        self.constructors.get(name).copied()
    }

    /// Return the set of registered command names.
    #[allow(dead_code)]
    pub fn command_names(&self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = self.constructors.keys().copied().collect();
        names.sort_unstable();
        names
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        build_default_registry()
    }
}

/// Build the default command registry with all known commands.
///
/// This function is the single compile-time source of truth for which
/// commands are available. Individual command constructors are added
/// here as they are migrated to service-owned `command.rs` files.
///
/// Commands that require per-invocation data (parsed flags, subcommand
/// selection) are still constructed in the parse layer; only stateless
/// or default-constructible commands are registered here.
pub fn build_default_registry() -> CommandRegistry {
    let mut registry = CommandRegistry::new();
    registry.register("help", crate::services::help::command::make_help_command);
    registry.register(
        "auth",
        crate::services::auth_command::command::make_auth_command,
    );
    registry.register(
        "config",
        crate::services::config::command::make_config_command,
    );
    registry.register("setup", crate::services::setup::command::make_setup_command);
    registry.register(
        "doctor",
        crate::services::doctor::command::make_doctor_command,
    );
    registry.register("hooks", crate::services::hooks::command::make_hooks_command);
    registry.register(
        "version",
        crate::services::version::command::make_version_command,
    );
    registry.register(
        "completion",
        crate::services::completion::command::make_completion_command,
    );
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestCommand;

    impl RuntimeCommand for TestCommand {
        fn name(&self) -> std::borrow::Cow<'_, str> {
            std::borrow::Cow::Borrowed("test")
        }

        fn execute(&self, _context: &AppContext) -> Result<String, ClassifiedError> {
            Ok("test output".to_string())
        }
    }

    fn make_test_command() -> RuntimeCommandHandle {
        Box::new(TestCommand)
    }

    #[test]
    fn registry_new_is_empty() {
        let registry = CommandRegistry::new();
        assert!(registry.command_names().is_empty());
        assert!(registry.get("test").is_none());
    }

    #[test]
    fn register_and_retrieve_command() {
        let mut registry = CommandRegistry::new();
        registry.register("test", make_test_command as CommandConstructor);

        let constructor = registry
            .get("test")
            .expect("should find registered command");
        let command = constructor();
        assert_eq!(command.name(), "test");
    }

    #[test]
    fn register_replaces_existing() {
        let mut registry = CommandRegistry::new();
        registry.register("test", make_test_command as CommandConstructor);
        registry.register("test", make_test_command as CommandConstructor);

        assert_eq!(registry.command_names().len(), 1);
    }

    #[test]
    fn default_registry_is_build_default() {
        let registry = CommandRegistry::default();
        // The default registry grows as commands are migrated to service-owned
        // command files. T02 registers "help"; T03–T04 add remaining commands.
        assert!(registry.command_names().contains(&"help"));
    }
}
