//! Shared subprocess hiding, non-interactive environment hardening, and Windows Job Object
//! process-tree cleanup for CLI probes.

use std::process::Command;

/// Apply Windows process creation flags that prevent flashing or visible console windows.
pub fn configure_hidden_cli_command(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW prevents creating a visible console window during background polling.
        cmd.creation_flags(0x0800_0000);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

/// Apply standard headless/non-interactive environment variables so CLI tools fail fast instead
/// of opening an interactive browser window or waiting on terminal prompts.
pub fn apply_headless_no_browser_env(cmd: &mut Command) {
    cmd.env("NO_BROWSER", "true")
        .env("CI", "true")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("TERM", "dumb")
        .env_remove("BROWSER");
}

/// Guard that binds a spawned child process (and its descendants, e.g. `conhost.exe` or loopback
/// sidecars) to an OS job/group on Windows so the entire process tree is cleaned up on timeout
/// or drop.
pub struct ChildProcessGuard {
    #[cfg(windows)]
    handle: Option<windows_sys::Win32::Foundation::HANDLE>,
}

impl ChildProcessGuard {
    /// Attach `child` to a kill-on-close Windows Job Object. Failures degrade gracefully to a
    /// no-op guard so callers never fail just because an outer sandbox already restricts job
    /// nesting on older Windows configurations.
    #[cfg(windows)]
    pub fn attach(child: &std::process::Child) -> Self {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };

        // SAFETY: CreateJobObjectW with null security attributes and null name creates a new
        // anonymous job object handle owned by this guard.
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() || job == INVALID_HANDLE_VALUE {
            return Self { handle: None };
        }

        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

        // SAFETY: `info` is a valid zero-initialized `JOBOBJECT_EXTENDED_LIMIT_INFORMATION`
        // matching `JobObjectExtendedLimitInformation`.
        let set_ok = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&raw const info).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if set_ok == 0 {
            // SAFETY: `job` is a valid non-null handle returned by `CreateJobObjectW`.
            unsafe {
                CloseHandle(job);
            }
            return Self { handle: None };
        }

        let proc_handle = child.as_raw_handle();
        // SAFETY: `job` and `proc_handle` are valid handles for the job object and live child.
        let assign_ok = unsafe { AssignProcessToJobObject(job, proc_handle) };
        if assign_ok == 0 {
            // SAFETY: `job` is a valid non-null handle returned by `CreateJobObjectW`.
            unsafe {
                CloseHandle(job);
            }
            return Self { handle: None };
        }

        Self { handle: Some(job) }
    }

    #[cfg(not(windows))]
    pub fn attach(_child: &std::process::Child) -> Self {
        Self {}
    }

    /// Terminate the entire process tree associated with this guard immediately.
    pub fn terminate_tree(&self) {
        #[cfg(windows)]
        if let Some(job) = self.handle {
            // SAFETY: `job` is an owned, still-open handle created by `CreateJobObjectW`.
            unsafe {
                windows_sys::Win32::System::JobObjects::TerminateJobObject(job, 1);
            }
        }
    }
}

#[cfg(windows)]
impl Drop for ChildProcessGuard {
    fn drop(&mut self) {
        if let Some(job) = self.handle.take() {
            // SAFETY: `job` is owned solely by this guard and closed once on drop.
            unsafe {
                windows_sys::Win32::System::JobObjects::TerminateJobObject(job, 1);
                windows_sys::Win32::Foundation::CloseHandle(job);
            }
        }
    }
}

/// Whether a Windows `QUERY_USER_NOTIFICATION_STATE` value indicates fullscreen game/presentation
/// mode (`QUNS_BUSY` = 2, `QUNS_RUNNING_D3D_FULL_SCREEN` = 3, `QUNS_PRESENTATION_MODE` = 4).
pub(crate) fn is_fullscreen_notification_state(state: i32) -> bool {
    matches!(state, 2..=4)
}

/// Returns `true` when Windows reports a fullscreen / D3D exclusive / presentation state so
/// background CLI polling can skip probing and avoid interrupting games or presentations.
pub fn is_fullscreen_busy() -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::{
            SHQueryUserNotificationState, QUERY_USER_NOTIFICATION_STATE,
        };
        let mut state: QUERY_USER_NOTIFICATION_STATE = 0;
        // SAFETY: `SHQueryUserNotificationState` writes to a valid stack-allocated enum integer.
        let hr = unsafe { SHQueryUserNotificationState(&mut state) };
        if hr == 0 {
            return is_fullscreen_notification_state(state);
        }
        false
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn fullscreen_notification_state_detects_busy_d3d_and_presentation_modes() {
        assert!(!is_fullscreen_notification_state(1)); // QUNS_NOT_PRESENT
        assert!(is_fullscreen_notification_state(2)); // QUNS_BUSY
        assert!(is_fullscreen_notification_state(3)); // QUNS_RUNNING_D3D_FULL_SCREEN
        assert!(is_fullscreen_notification_state(4)); // QUNS_PRESENTATION_MODE
        assert!(!is_fullscreen_notification_state(5)); // QUNS_ACCEPTS_NOTIFICATIONS
        assert!(!is_fullscreen_notification_state(6)); // QUNS_QUIET_TIME
        assert!(!is_fullscreen_notification_state(7)); // QUNS_APP
    }

    #[test]
    fn apply_headless_no_browser_env_sets_non_interactive_vars_and_removes_browser() {
        let mut cmd = Command::new("placeholder");
        cmd.env("BROWSER", "chrome.exe");
        apply_headless_no_browser_env(&mut cmd);

        let envs: Vec<(String, Option<String>)> = cmd
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|val| OsStr::to_string_lossy(val).into_owned()),
                )
            })
            .collect();

        assert!(envs.contains(&("NO_BROWSER".to_string(), Some("true".to_string()))));
        assert!(envs.contains(&("CI".to_string(), Some("true".to_string()))));
        assert!(envs.contains(&("GIT_TERMINAL_PROMPT".to_string(), Some("0".to_string()))));
        assert!(envs.contains(&("TERM".to_string(), Some("dumb".to_string()))));
        assert!(envs.contains(&("BROWSER".to_string(), None)));
    }

    #[cfg(windows)]
    #[test]
    fn child_process_guard_terminates_spawned_tree_promptly() {
        use std::process::Stdio;
        use std::time::{Duration, Instant};

        let mut cmd = Command::new("cmd.exe");
        cmd.args(["/d", "/c", "ping -n 20 127.0.0.1 >nul"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        configure_hidden_cli_command(&mut cmd);

        let mut child = cmd.spawn().expect("spawn ping subprocess");
        let guard = ChildProcessGuard::attach(&child);
        let started = Instant::now();
        guard.terminate_tree();
        let status = child.wait().expect("wait terminated child");
        drop(guard);

        assert!(!status.success());
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
