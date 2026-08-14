use super::process_args::{wide_command_line, wide_environment, wide_path};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, GetExitCodeProcess, ResumeThread, TerminateProcess, WaitForSingleObject,
    CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, STARTUPINFOW,
};

pub(super) struct AutoStartedChild {
    process: usize,
    job: usize,
}

impl AutoStartedChild {
    fn process_handle(&self) -> HANDLE {
        self.process as HANDLE
    }

    fn job_handle(&self) -> HANDLE {
        self.job as HANDLE
    }

    pub(super) fn try_wait(&mut self) -> Option<u32> {
        let status = unsafe { WaitForSingleObject(self.process_handle(), 0) };
        if status == WAIT_TIMEOUT {
            return None;
        }
        let mut exit_code = 1;
        if status == WAIT_OBJECT_0 {
            let _ = unsafe { GetExitCodeProcess(self.process_handle(), &mut exit_code) };
        }
        Some(exit_code)
    }

    pub(super) fn stop(&mut self) {
        unsafe {
            if self.job != 0 {
                let _ = TerminateJobObject(self.job_handle(), 1);
                let _ = WaitForSingleObject(self.process_handle(), 5_000);
            } else {
                let _ = TerminateProcess(self.process_handle(), 1);
                let _ = WaitForSingleObject(self.process_handle(), 5_000);
            }
        }
    }
}

impl Drop for AutoStartedChild {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.process_handle());
            if self.job != 0 {
                let _ = CloseHandle(self.job_handle());
            }
        }
    }
}

pub(super) fn spawn_auto_started(
    executable: &std::path::Path,
    base_dir: &std::path::Path,
    config_files: &[String],
    terminate_on_exit: bool,
) -> Result<AutoStartedChild, String> {
    let mut application = wide_path(executable);
    let mut command_line = wide_command_line(executable, config_files);
    let mut environment = wide_environment();
    let current_directory = wide_path(base_dir);
    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    let mut process_info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let creation_flags = CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT;

    let created = unsafe {
        CreateProcessW(
            application.as_mut_ptr(),
            command_line.as_mut_ptr(),
            null(),
            null(),
            0,
            creation_flags,
            environment.as_mut_ptr().cast(),
            current_directory.as_ptr(),
            &startup,
            &mut process_info,
        )
    };
    if created == 0 {
        return Err(format!("CreateProcessW failed ({})", unsafe {
            GetLastError()
        }));
    }

    let job = if terminate_on_exit {
        let job = unsafe { CreateJobObjectW(null(), null()) };
        if job.is_null() {
            let error = unsafe { GetLastError() };
            unsafe {
                let _ = TerminateProcess(process_info.hProcess, 1);
                let _ = CloseHandle(process_info.hThread);
                let _ = CloseHandle(process_info.hProcess);
            }
            return Err(format!("CreateJobObjectW failed ({error})"));
        }

        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        } != 0;
        let assigned =
            configured && unsafe { AssignProcessToJobObject(job, process_info.hProcess) } != 0;
        if !assigned {
            let error = unsafe { GetLastError() };
            unsafe {
                let _ = TerminateProcess(process_info.hProcess, 1);
                let _ = CloseHandle(process_info.hThread);
                let _ = CloseHandle(process_info.hProcess);
                let _ = CloseHandle(job);
            }
            return Err(format!("AM Daemon job setup failed ({error})"));
        }
        job
    } else {
        null_mut()
    };

    if unsafe { ResumeThread(process_info.hThread) } == u32::MAX {
        let error = unsafe { GetLastError() };
        unsafe {
            if !job.is_null() {
                let _ = TerminateJobObject(job, 1);
            } else {
                let _ = TerminateProcess(process_info.hProcess, 1);
            }
            let _ = CloseHandle(process_info.hThread);
            let _ = CloseHandle(process_info.hProcess);
            if !job.is_null() {
                let _ = CloseHandle(job);
            }
        }
        return Err(format!("ResumeThread failed ({error})"));
    }

    unsafe {
        let _ = CloseHandle(process_info.hThread);
    }

    Ok(AutoStartedChild {
        process: process_info.hProcess as usize,
        job: job as usize,
    })
}
