//! Canonical controller and per-session Process templates.

/// Process domain selected by a shell-terminal template.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateDomain {
    /// The Provider controller is system-domain.
    System,
    /// Each shell supervisor is user-domain.
    User,
}

/// Fixed sandbox properties exposed to the process-template compiler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SandboxProfile {
    no_new_privileges: bool,
    start_root: bool,
    read_only_root: bool,
    ambient_credentials: bool,
}

impl SandboxProfile {
    /// Return whether the template forbids inherited ambient credentials.
    pub const fn denies_ambient_credentials(&self) -> bool {
        !self.ambient_credentials && self.no_new_privileges && !self.start_root
    }

    /// Return whether the process sees a read-only root filesystem.
    pub const fn read_only_root(&self) -> bool {
        self.read_only_root
    }
}

/// A canonical Process template with no raw argv, path, uid, or cgroup data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessTemplate {
    domain: TemplateDomain,
    sandbox: SandboxProfile,
    adopt_on_restart: bool,
    restart_automatically: bool,
}

impl ProcessTemplate {
    /// Return the Provider controller template.
    pub const fn controller() -> Self {
        Self {
            domain: TemplateDomain::System,
            sandbox: SandboxProfile {
                no_new_privileges: true,
                start_root: false,
                read_only_root: true,
                ambient_credentials: false,
            },
            adopt_on_restart: true,
            restart_automatically: true,
        }
    }

    /// Return the per-session workload-user supervisor template.
    pub const fn session_supervisor() -> Self {
        Self {
            domain: TemplateDomain::User,
            sandbox: SandboxProfile {
                no_new_privileges: true,
                start_root: false,
                read_only_root: true,
                ambient_credentials: false,
            },
            adopt_on_restart: true,
            restart_automatically: false,
        }
    }

    /// Return the Process domain.
    pub const fn domain(&self) -> TemplateDomain {
        self.domain
    }

    /// Borrow the constrained sandbox profile.
    pub const fn sandbox(&self) -> &SandboxProfile {
        &self.sandbox
    }

    /// Return whether restart adoption is required.
    pub const fn adopts_on_restart(&self) -> bool {
        self.adopt_on_restart
    }

    /// Return whether a terminal supervisor restarts automatically.
    pub const fn restarts_automatically(&self) -> bool {
        self.restart_automatically
    }
}
