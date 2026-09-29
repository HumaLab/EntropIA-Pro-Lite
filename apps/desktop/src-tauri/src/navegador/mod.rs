//! The embedded browser ("Navegador"). Web content is untrusted: nothing in
//! this module hands it IPC, and every address it may load goes through
//! [`url_policy`] first.

// Nothing calls the policy until the viewer lands.
#[allow(dead_code)]
pub mod url_policy;
