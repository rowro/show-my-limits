//! Keep every helper worker owned by the app. Windows one-file helpers spawn
//! a second process: killing just the launcher leaves capture running.
use std::{
    io,
    ops::{Deref, DerefMut},
    process::{Child, Command},
};

pub struct OwnedChild {
    child: Child,
    #[cfg(windows)]
    job: windows::Job,
}
impl OwnedChild {
    pub fn spawn(command: &mut Command) -> io::Result<Self> {
        #[cfg(windows)]
        {
            windows::spawn(command)
        }
        #[cfg(not(windows))]
        {
            Ok(Self {
                child: command.spawn()?,
            })
        }
    }
    pub fn kill(&mut self) -> io::Result<()> {
        #[cfg(windows)]
        {
            self.job.terminate()
        }
        #[cfg(not(windows))]
        {
            self.child.kill()
        }
    }
}
impl Deref for OwnedChild {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.child
    }
}
impl DerefMut for OwnedChild {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.child
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::{
        mem::size_of,
        os::windows::{
            io::{AsRawHandle, FromRawHandle, OwnedHandle},
            process::CommandExt,
        },
        ptr,
    };
    use windows_sys::Win32::{
        Foundation::INVALID_HANDLE_VALUE,
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First,
                Thread32Next,
            },
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                SetInformationJobObject, TerminateJobObject,
            },
            Threading::{
                CREATE_NO_WINDOW, CREATE_SUSPENDED, GetProcessIdOfThread, OpenThread, ResumeThread,
                THREAD_QUERY_LIMITED_INFORMATION, THREAD_SUSPEND_RESUME,
            },
        },
    };
    // CreateJobObject(NULL attributes) makes this handle non-inheritable. Only
    // the app owns it; even an abrupt app exit closes it and kills its workers.
    pub struct Job(OwnedHandle);
    impl Job {
        fn new() -> io::Result<Self> {
            let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if unsafe {
                SetInformationJobObject(
                    handle.as_raw_handle(),
                    JobObjectExtendedLimitInformation,
                    (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(Self(handle))
        }
        pub fn terminate(&self) -> io::Result<()> {
            if unsafe { TerminateJobObject(self.0.as_raw_handle(), 1) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
    }
    pub(super) fn spawn(command: &mut Command) -> io::Result<OwnedChild> {
        let job = Job::new()?;
        // Assignment after an ordinary spawn races with PyInstaller spawning its
        // worker. Suspend the initial thread until membership is guaranteed.
        let mut child = command
            .creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED)
            .spawn()?;
        let attach =
            unsafe { AssignProcessToJobObject(job.0.as_raw_handle(), child.as_raw_handle()) };
        if attach == 0 {
            let error = io::Error::last_os_error();
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                error.kind(),
                format!("Cannot own helper process tree: {error}"),
            ));
        }
        if let Err(error) = resume_owned_process(child.id()) {
            let _ = job.terminate();
            let _ = child.wait();
            return Err(error);
        }
        Ok(OwnedChild { child, job })
    }
    fn resume_owned_process(pid: u32) -> io::Result<()> {
        // std::process::Child deliberately doesn't expose the initial thread
        // handle. Its live process handle pins the PID while we locate threads.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot) };
        let mut entry = THREADENTRY32 {
            dwSize: size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        let mut found = false;
        let mut more = unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) } != 0;
        while more {
            if entry.th32OwnerProcessID == pid {
                let thread = unsafe {
                    OpenThread(
                        THREAD_QUERY_LIMITED_INFORMATION | THREAD_SUSPEND_RESUME,
                        0,
                        entry.th32ThreadID,
                    )
                };
                if thread.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let thread = unsafe { OwnedHandle::from_raw_handle(thread) };
                // A snapshot thread ID could have been recycled; validate the
                // live handle before touching it. Never resume another app.
                if unsafe { GetProcessIdOfThread(thread.as_raw_handle()) } != pid {
                    more = unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) } != 0;
                    continue;
                }
                if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
                    return Err(io::Error::last_os_error());
                }
                found = true;
            }
            more = unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) } != 0;
        }
        if !found {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "Suspended helper thread not found",
            ));
        }
        Ok(())
    }
}

#[cfg(all(windows, test))]
mod tests {
    use super::*;
    use std::{
        fs,
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
        path::{Path, PathBuf},
        process::Stdio,
        thread,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            WaitForSingleObject,
        },
    };
    const DIRECTORY: &str = "SML_PROCESS_TREE_FIXTURE_DIR";
    fn fixture_dir() -> Option<PathBuf> {
        std::env::var_os(DIRECTORY).map(PathBuf::from)
    }
    fn command(name: &str, directory: &Path) -> Command {
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.args([
            "--exact",
            &format!("process_tree::tests::{name}"),
            "--ignored",
            "--nocapture",
        ])
        .env(DIRECTORY, directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
        cmd
    }
    #[test]
    #[ignore = "subprocess fixture; runs only with a private test directory"]
    fn fixture_worker() {
        let Some(dir) = fixture_dir() else {
            return;
        };
        fs::write(dir.join("worker.ready"), std::process::id().to_string()).unwrap();
        thread::sleep(Duration::from_secs(120));
    }
    #[test]
    #[ignore = "subprocess fixture; runs only with a private test directory"]
    fn fixture_launcher() {
        let Some(dir) = fixture_dir() else {
            return;
        };
        let _worker = command("fixture_worker", &dir).spawn().unwrap();
        fs::write(dir.join("launcher.ready"), std::process::id().to_string()).unwrap();
        thread::sleep(Duration::from_secs(120));
    }
    #[test]
    #[ignore = "subprocess fixture; runs only with a private test directory"]
    fn fixture_app_owner() {
        let Some(dir) = fixture_dir() else {
            return;
        };
        let _capture = OwnedChild::spawn(&mut command("fixture_launcher", &dir)).unwrap();
        thread::sleep(Duration::from_secs(120));
    }
    struct Control(Child);
    impl Drop for Control {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    fn running_handle(dir: &Path, name: &str) -> OwnedHandle {
        let path = dir.join(format!("{name}.ready"));
        let deadline = Instant::now() + Duration::from_secs(15);
        let pid = loop {
            if let Some(pid) = fs::read_to_string(&path)
                .ok()
                .and_then(|v| v.parse::<u32>().ok())
            {
                break pid;
            }
            assert!(
                Instant::now() < deadline,
                "fixture did not become ready: {name}"
            );
            thread::sleep(Duration::from_millis(25));
        };
        let handle = unsafe {
            OpenProcess(
                PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                pid,
            )
        };
        assert!(!handle.is_null(), "could not open running fixture {pid}");
        let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
        assert_eq!(
            unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) },
            WAIT_TIMEOUT
        );
        handle
    }
    fn assert_exited(handle: &OwnedHandle) {
        assert_eq!(
            unsafe { WaitForSingleObject(handle.as_raw_handle(), 5000) },
            WAIT_OBJECT_0,
            "owned worker survived stop/owner exit"
        );
    }
    #[test]
    fn stop_drop_and_app_crash_release_the_owned_tree_without_killing_a_sibling() {
        let root = tempfile::tempdir().unwrap();
        let sibling_dir = root.path().join("sibling");
        fs::create_dir_all(&sibling_dir).unwrap();
        let mut sibling = Control(command("fixture_worker", &sibling_dir).spawn().unwrap());
        let sibling_handle = running_handle(&sibling_dir, "worker");
        for stop in [false, true] {
            let dir = root.path().join(if stop { "stop" } else { "drop" });
            fs::create_dir_all(&dir).unwrap();
            let mut capture = OwnedChild::spawn(&mut command("fixture_launcher", &dir)).unwrap();
            let launcher = running_handle(&dir, "launcher");
            let worker = running_handle(&dir, "worker");
            if stop {
                capture.kill().unwrap();
            }
            drop(capture);
            assert_exited(&launcher);
            assert_exited(&worker);
            assert!(sibling.0.try_wait().unwrap().is_none());
        }
        let dir = root.path().join("crash");
        fs::create_dir_all(&dir).unwrap();
        let mut app_owner = Control(command("fixture_app_owner", &dir).spawn().unwrap());
        let launcher = running_handle(&dir, "launcher");
        let worker = running_handle(&dir, "worker");
        // Terminate the app without running any of its Rust cleanup code.
        app_owner.0.kill().unwrap();
        app_owner.0.wait().unwrap();
        assert_exited(&launcher);
        assert_exited(&worker);
        assert_eq!(
            unsafe { WaitForSingleObject(sibling_handle.as_raw_handle(), 0) },
            WAIT_TIMEOUT
        );
    }
}
