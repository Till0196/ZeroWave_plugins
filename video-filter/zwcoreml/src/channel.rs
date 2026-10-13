// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! メッセージのチャネル：標準入力と標準出力を兼ねる一本の unix ソケット（fd は SCM_RIGHTS で付く）と、
//! metal の方式で IOSurface のポートを受ける mach の受信ポート。中身の読み書きは zwsdk の物。

use std::collections::VecDeque;
use std::ffi::{CString, c_char, c_void};
use std::fs::File;
use std::io::Write;
use std::mem::size_of;
use std::os::fd::{FromRawFd, OwnedFd};
use std::sync::{Arc, Mutex};

use zwsdk::video::process as wire;

#[derive(Debug, Clone)]
pub struct Replier {
    out: Arc<Mutex<File>>,
}

impl Replier {
    pub fn reply(&self, body: &[u8]) {
        self.send(&wire::reply(Ok(body)));
    }

    pub fn fail(&self, why: &str) {
        self.send(&wire::reply(Err(why)));
    }

    fn send(&self, message: &[u8]) {
        let mut out = self.out.lock().unwrap_or_else(|e| e.into_inner());
        let mut rest = message;
        while !rest.is_empty() {
            match out.write(rest) {
                Ok(0) | Err(_) => return,
                Ok(put) => rest = &rest[put..],
            }
        }
    }
}

#[derive(Debug)]
pub struct Channel {
    fds: VecDeque<OwnedFd>,
    port: Option<u32>,
}

impl Channel {
    /// ほかの所が標準出力へ書いても混ざらないよう、標準出力を標準エラーへ向け直す。
    pub fn take_stdio() -> (Self, Replier) {
        // SAFETY: 本体が先に閉じても、書いたところで落ちないように。fd の複製とリダイレクト。
        let out = unsafe {
            libc::signal(libc::SIGPIPE, libc::SIG_IGN);
            let out = libc::dup(libc::STDOUT_FILENO);
            libc::dup2(libc::STDERR_FILENO, libc::STDOUT_FILENO);
            OwnedFd::from_raw_fd(out)
        };
        let replier = Replier {
            out: Arc::new(Mutex::new(File::from(out))),
        };
        (
            Self {
                fds: VecDeque::new(),
                port: None,
            },
            replier,
        )
    }

    pub fn receive(&mut self) -> Option<(u32, Vec<u8>)> {
        let mut head = [0u8; 8];
        self.read_exact(&mut head)?;
        let (op, len) = wire::head(head);
        let mut body = vec![0u8; len];
        self.read_exact(&mut body)?;
        Some((op, body))
    }

    /// いつも recvmsg で読み、付いてきた fd を保持する（read だと捨てられる）。
    fn read_exact(&mut self, mut into: &mut [u8]) -> Option<()> {
        const SPACE: usize = 64;
        while !into.is_empty() {
            let mut iov = libc::iovec {
                iov_base: into.as_mut_ptr().cast(),
                iov_len: into.len(),
            };
            let mut control = [0u64; SPACE / 8];
            // SAFETY: ローカルの iovec と制御の領域で受ける。付いてきた fd はこちらの物になる。
            let got = unsafe {
                let mut header: libc::msghdr = std::mem::zeroed();
                header.msg_iov = &raw mut iov;
                header.msg_iovlen = 1;
                header.msg_control = control.as_mut_ptr().cast();
                header.msg_controllen = SPACE as _;
                let got = libc::recvmsg(libc::STDIN_FILENO, &raw mut header, 0);
                if got > 0 {
                    let mut c = libc::CMSG_FIRSTHDR(&raw const header);
                    while !c.is_null() {
                        if (*c).cmsg_level == libc::SOL_SOCKET && (*c).cmsg_type == libc::SCM_RIGHTS
                        {
                            let count = ((*c).cmsg_len as usize - libc::CMSG_LEN(0) as usize)
                                / size_of::<i32>();
                            let data = libc::CMSG_DATA(c).cast::<i32>();
                            for i in 0..count {
                                self.fds
                                    .push_back(OwnedFd::from_raw_fd(data.add(i).read_unaligned()));
                            }
                        }
                        c = libc::CMSG_NXTHDR(&raw const header, c);
                    }
                }
                got
            };
            if got <= 0 {
                return None;
            }
            into = &mut into[got as usize..];
        }
        Some(())
    }

    pub fn take_fd(&mut self) -> Result<OwnedFd, String> {
        self.fds
            .pop_front()
            .ok_or_else(|| String::from("SHARE に fd が付いていない"))
    }

    pub fn check_in(&mut self, service: &str) -> Result<(), String> {
        let name = CString::new(service).map_err(|e| e.to_string())?;
        let mut port = 0u32;
        // SAFETY: 名前は C の文字列、出力はローカル変数。
        let status = unsafe { bootstrap_check_in(bootstrap_port, name.as_ptr(), &raw mut port) };
        if status != 0 {
            return Err(format!(
                "bootstrap_check_in ができない：{service}（{status}）"
            ));
        }
        self.port = Some(port);
        Ok(())
    }

    pub fn take_port(&mut self, what: u32, index: u32) -> Result<u32, String> {
        let port = self
            .port
            .ok_or_else(|| String::from("mach の受信ポートが無い"))?;
        let mut message = PortMessage::default();
        // SAFETY: ローカルのメッセージの大きさで受ける。
        let status = unsafe {
            mach_msg(
                (&raw mut message).cast(),
                MACH_RCV_MSG | MACH_RCV_TIMEOUT,
                0,
                size_of::<PortMessage>() as u32,
                port,
                5000,
                0,
            )
        };
        if status != 0 {
            return Err(format!(
                "IOSurface のポートが来ない（mach_msg {status:#x}）"
            ));
        }
        let id = message.id;
        let descriptors = message.descriptors;
        if id as u32 != wire::port_id(what, index) || descriptors != 1 {
            return Err(String::from("IOSurface のポートのメッセージが違う"));
        }
        Ok(message.name)
    }
}

pub fn release_port(port: u32) {
    // SAFETY: こちらが持つ権利を一つ手放す。
    unsafe {
        mach_port_deallocate(mach_task_self_, port);
    }
}

const MACH_RCV_MSG: i32 = 0x0000_0002;
const MACH_RCV_TIMEOUT: i32 = 0x0000_0100;

/// ポートの記述子を一つ持つ mach のメッセージ（4 バイト境界にパックした並び）。
#[repr(C, packed(4))]
#[derive(Default)]
struct PortMessage {
    bits: u32,
    size: u32,
    remote: u32,
    local: u32,
    voucher: u32,
    id: i32,
    descriptors: u32,
    name: u32,
    pad1: u32,
    pad2: u16,
    disposition: u8,
    kind: u8,
    trailer_type: u32,
    trailer_size: u32,
}

unsafe extern "C" {
    static bootstrap_port: u32;
    static mach_task_self_: u32;
    fn bootstrap_check_in(bp: u32, name: *const c_char, sp: *mut u32) -> i32;
    fn mach_msg(
        msg: *mut c_void,
        option: i32,
        send_size: u32,
        rcv_size: u32,
        rcv_name: u32,
        timeout: u32,
        notify: u32,
    ) -> i32;
    fn mach_port_deallocate(task: u32, name: u32) -> i32;
}

#[derive(Debug)]
pub struct Mapping {
    data: *mut u8,
    size: usize,
}

// SAFETY: マップしたアドレスを持つだけ。読み書きは一つずつ処理するジョブの中だけ。
unsafe impl Send for Mapping {}

impl Mapping {
    pub fn map(fd: OwnedFd, size: u64) -> Result<Self, String> {
        let size = usize::try_from(size).map_err(|e| e.to_string())?;
        // SAFETY: 大きさの分を読み書きで共有してマップする。
        let view = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                std::os::fd::AsRawFd::as_raw_fd(&fd),
                0,
            )
        };
        drop(fd);
        if view == libc::MAP_FAILED {
            return Err(String::from("共有メモリをマップできない"));
        }
        Ok(Self {
            data: view.cast(),
            size,
        })
    }

    pub fn data(&self) -> *mut u8 {
        self.data
    }

    pub fn size(&self) -> usize {
        self.size
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: map でマップした物を外す。
        unsafe {
            libc::munmap(self.data.cast(), self.size);
        }
    }
}
