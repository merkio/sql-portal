use crate::domain::access::identity::Principal;
use crate::domain::scripts::ScriptDefinition;

/// Decides whether a principal may use a script. The default implementation
/// checks permissions (see `application::RequiredPermissions`); a dependent crate
/// can supply its own and pass it to `bootstrap::build_app_with` in `Overrides::script_access`.
pub trait ScriptAccessRule: Send + Sync {
    fn can_access(&self, principal: &Principal, script: &ScriptDefinition) -> bool;
}
