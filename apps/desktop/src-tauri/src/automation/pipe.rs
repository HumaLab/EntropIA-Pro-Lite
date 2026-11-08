//! Windows transport only; HTTP authentication and MCP stay in `server`.
use super::*;
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;

pub(super) struct Listener {
    pub(super) name: String,
    sddl: String,
    pending: NamedPipeServer,
}

impl Listener {
    pub(super) fn bind() -> Result<Self, String> {
        let user = crate::automation::private_acl::current_user_sid()?;
        // Protected DACL: exactly the process user, no SYSTEM or inherited ACEs.
        let sddl = format!("D:P(A;;GA;;;{user})");
        let name = format!(r"\\.\pipe\entropia-agent-{}", proof::mint_instance_key()?);
        let pending = create(&name, &sddl, true)
            .map_err(|e| format!("could not create assistant pipe: {e}"))?;
        Ok(Self {
            name,
            sddl,
            pending,
        })
    }

    pub(super) async fn serve(mut self, service: GuardedEndpoint, cancel: CancellationToken) {
        let mut failures: u32 = 0;
        loop {
            let connected = tokio::select! {
                biased;
                _ = cancel.cancelled() => return,
                result = self.pending.connect() => result,
            };
            if connected.is_ok() {
                // Create the next instance before handing this one to HTTP. There
                // is always a server handle retaining ownership of the pipe name.
                match create(&self.name, &self.sddl, false) {
                    Ok(next) => {
                        let stream = std::mem::replace(&mut self.pending, next);
                        tokio::spawn(serve_connection(
                            TokioIo::new(stream),
                            service.clone(),
                            cancel.clone(),
                        ));
                        failures = 0;
                        continue;
                    }
                    Err(error) => {
                        log::warn!(target: "automation", "could not replenish assistant pipe: {error}")
                    }
                }
            }
            failures = failures.saturating_add(1);
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep(accept_backoff(failures)) => {}
            }
        }
    }
}

fn create(name: &str, sddl: &str, first: bool) -> std::io::Result<NamedPipeServer> {
    let wide: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
    let mut descriptor = std::ptr::null_mut();
    // SAFETY: NUL-terminated input and valid out pointer. Windows allocates the
    // descriptor; LocalFree below runs on both success and failure of creation.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    // SAFETY: attributes and the descriptor remain valid throughout CreateNamedPipe.
    let result = unsafe {
        ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(
                name,
                (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
            )
    };
    // SAFETY: allocated by the SDDL conversion, and no longer needed by Windows.
    unsafe {
        LocalFree(descriptor);
    }
    result
}
