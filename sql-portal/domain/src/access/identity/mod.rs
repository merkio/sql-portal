//! Identity: who a user is, which roles and permissions they resolve to.
//!
//! `user` holds what an identity provider asserts, `principal` what the
//! application knows about a signed-in user, and `policy` maps the first to the
//! second through roles.

pub mod error;
pub mod grants;
pub mod policy;
pub mod principal;
pub mod user;

pub use error::AuthError;
pub use grants::Grants;
pub use policy::AccessPolicy;
pub use principal::Principal;
pub use user::{AuthProvider, User};
