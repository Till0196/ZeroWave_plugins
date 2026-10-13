// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! zwcoreml：RIFE を CoreML（ANE）と Metal で動かす、本体の外部プロセスの補間の実装
//! （プロトコルは zerowave の sdk/src/video/process.md、読み書きは zwsdk::video::process）。
//! mac は親と一緒に子を終わらせる仕組みが無いので、親の終わりを kqueue で見て自分で終わる。

#[cfg(target_os = "macos")]
mod channel;
#[cfg(target_os = "macos")]
mod interpolator;
#[cfg(target_os = "macos")]
mod report;

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("zwcoreml は macOS だけ");
    std::process::exit(2);
}

#[cfg(target_os = "macos")]
fn main() {
    std::process::exit(mac::main());
}

#[cfg(target_os = "macos")]
mod mac {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Instant;

    use objc2::rc::autoreleasepool;
    use zwsdk::video::process::{self as wire, Format, MetalId, Offer, Open, Request, Way};

    use crate::channel::{Channel, Replier};
    use crate::interpolator::{Interpolator, Options, bench_run, units_of};
    use crate::report::log_line;

    const USAGE: i32 = 2;
    const USAGE_TEXT: &str = "使い方：zwcoreml serve <フォルダー> <モデル名> [計算=all|gpu|ane|cpu]\n\
        \x20      zwcoreml bench <フォルダー> <モデル名> <幅> <高さ> [回数=100] [計算=all] [同時=1]";

    static PREPARING: AtomicBool = AtomicBool::new(false);

    pub fn main() -> i32 {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let result = match args.first().map(String::as_str) {
            Some("serve") if args.len() >= 3 => serve(&args),
            Some("bench") if args.len() >= 5 => bench(&args),
            _ => {
                log_line(USAGE_TEXT);
                return USAGE;
            }
        };
        match result {
            Ok(()) => 0,
            Err(why) => {
                log_line(&why);
                1
            }
        }
    }

    fn number(text: Option<&String>, default: u32) -> u32 {
        text.and_then(|t| t.parse().ok()).unwrap_or(default)
    }

    fn options(args: &[String], units_at: usize) -> Options {
        Options {
            models: PathBuf::from(&args[1]),
            name: args[2].clone(),
            units: units_of(args.get(units_at).map_or("all", String::as_str)),
        }
    }

    fn watch_parent() {
        // SAFETY: kqueue で親の終わりを待ち、標準入力を覗くだけ。終わるときは _exit。
        unsafe {
            let parent = libc::getppid();
            let queue = libc::kqueue();
            let mut change: libc::kevent = std::mem::zeroed();
            change.ident = parent as usize;
            change.filter = libc::EVFILT_PROC;
            change.flags = libc::EV_ADD | libc::EV_ONESHOT;
            change.fflags = libc::NOTE_EXIT;
            if parent == 1
                || queue < 0
                || libc::kevent(queue, &change, 1, std::ptr::null_mut(), 0, std::ptr::null()) < 0
            {
                log_line("親が見えない（もう終わった）ので終わる");
                libc::_exit(0);
            }
            loop {
                let mut event: libc::kevent = std::mem::zeroed();
                let wait = libc::timespec {
                    tv_sec: 0,
                    tv_nsec: 200_000_000,
                };
                if libc::kevent(queue, std::ptr::null(), 0, &mut event, 1, &wait) > 0 {
                    log_line("親が終わったので終わる");
                    libc::_exit(0);
                }
                if !PREPARING.load(Ordering::Relaxed) {
                    continue;
                }
                let mut input = libc::pollfd {
                    fd: libc::STDIN_FILENO,
                    events: libc::POLLIN,
                    revents: 0,
                };
                let mut byte = 0u8;
                if libc::poll(&mut input, 1, 0) > 0
                    && libc::recv(
                        libc::STDIN_FILENO,
                        (&raw mut byte).cast(),
                        1,
                        libc::MSG_PEEK | libc::MSG_DONTWAIT,
                    ) == 0
                {
                    log_line("用意の間に標準入力が閉じたので終わる");
                    libc::_exit(0);
                }
            }
        }
    }

    struct Server {
        options: Options,
        channel: Channel,
        replier: Replier,
        interpolator: Option<Interpolator>,
    }

    impl Server {
        fn serve(&mut self) {
            while let Some((op, body)) = self.channel.receive() {
                autoreleasepool(|_| {
                    if let Err(why) = self.handle(op, &body) {
                        self.replier.fail(&why);
                    }
                });
            }
        }

        fn handle(&mut self, op: u32, body: &[u8]) -> Result<(), String> {
            match Request::decode(op, body)? {
                Request::Open(open) => self.open(&open),
                Request::Share(share) => {
                    let interpolator = self.interpolator.as_mut().ok_or("SHARE が OPEN より先")?;
                    interpolator.share(&share, &mut self.channel)?;
                    self.replier.reply(&[]);
                    Ok(())
                }
                Request::Run(run) => {
                    let interpolator = self.interpolator.as_mut().ok_or("RUN が OPEN より先")?;
                    if !interpolator.asynchronous() {
                        let ran = interpolator.run(&run)?;
                        self.replier.reply(&ran.encode());
                        return Ok(());
                    }
                    // 要求して戻り、次のメッセージを受ける。応答は書き終えたら CoreML か Metal のスレッドから。
                    let (done, failed) = (self.replier.clone(), self.replier.clone());
                    interpolator.run_async(
                        &run,
                        Arc::new(move |ran: wire::Ran| done.reply(&ran.encode())),
                        Arc::new(move |why: String| failed.fail(&why)),
                    )
                }
            }
        }

        fn open(&mut self, open: &Open) -> Result<(), String> {
            self.interpolator = None;
            PREPARING.store(true, Ordering::Relaxed);
            let made = Interpolator::new(&self.options, open);
            PREPARING.store(false, Ordering::Relaxed);
            let interpolator = self.interpolator.insert(made?);
            if interpolator.chosen().way == Way::Metal as u32 {
                self.channel.check_in(interpolator.service())?;
            }
            self.replier.reply(&interpolator.chosen().encode());
            Ok(())
        }
    }

    fn serve(args: &[String]) -> Result<(), String> {
        let options = options(args, 3);
        // チャネルを先に取る（取ったあとの標準出力は標準エラーへ向く）。
        let (channel, replier) = Channel::take_stdio();
        std::thread::spawn(watch_parent);
        Server {
            options,
            channel,
            replier,
            interpolator: None,
        }
        .serve();
        Ok(())
    }

    fn bench(args: &[String]) -> Result<(), String> {
        let options = options(args, 6);
        let runs = number(args.get(5), 100).max(1);
        let open = Open {
            width: number(args.get(3), 0),
            height: number(args.get(4), 0),
            jobs: number(args.get(7), 1).max(1),
            offers: vec![Offer::new(
                Way::Metal,
                MetalId {
                    registry_id: 0,
                    service: String::from("bench"),
                }
                .encode(),
            )],
            formats: vec![Format::F16 as u32],
        };
        let opened = Instant::now();
        let mut interpolator = Interpolator::new(&options, &open)?;
        log_line(&format!("用意 {:.1} 秒", opened.elapsed().as_secs_f64()));
        interpolator.use_own_surfaces(interpolator.chosen().jobs)?;
        let jobs = interpolator.jobs();
        let began = Instant::now();
        let times = if jobs == 1 {
            let mut times = Vec::new();
            for i in 0..runs {
                let ran = autoreleasepool(|_| interpolator.run(&bench_run(i, jobs)))?;
                times.push(ran.gpu_ms);
            }
            times
        } else {
            bench_async(&interpolator, runs, jobs)?
        };
        let wall = began.elapsed().as_secs_f64() * 1000.0;
        let mut times = times;
        times.sort_by(f32::total_cmp);
        log_line(&format!(
            "{runs} 回（同時に {jobs}）：一枚あたり {:.1} ms、一つのジョブは中央値 {:.1} ms・最小 {:.1} ms・最大 {:.1} ms",
            wall / f64::from(runs),
            times[times.len() / 2],
            times[0],
            times[times.len() - 1]
        ));
        Ok(())
    }

    fn bench_async(interpolator: &Interpolator, runs: u32, jobs: u32) -> Result<Vec<f32>, String> {
        #[derive(Default)]
        struct Board {
            times: Vec<f32>,
            busy: Vec<bool>,
            failure: Option<String>,
        }
        let state = Arc::new((
            Mutex::new(Board {
                busy: vec![false; jobs as usize],
                ..Board::default()
            }),
            Condvar::new(),
        ));
        fn lock(state: &(Mutex<Board>, Condvar)) -> std::sync::MutexGuard<'_, Board> {
            state.0.lock().unwrap_or_else(|e| e.into_inner())
        }
        for i in 0..runs {
            let run = bench_run(i, jobs);
            let at = run.middle as usize;
            {
                let board = lock(&state);
                let mut board = state
                    .1
                    .wait_while(board, |b| b.busy[at] && b.failure.is_none())
                    .unwrap_or_else(|e| e.into_inner());
                if let Some(why) = &board.failure {
                    return Err(why.clone());
                }
                board.busy[at] = true;
            }
            let (done, failed) = (state.clone(), state.clone());
            autoreleasepool(|_| {
                interpolator.run_async(
                    &run,
                    Arc::new(move |ran: wire::Ran| {
                        let mut board = lock(&done);
                        board.times.push(ran.gpu_ms);
                        board.busy[at] = false;
                        done.1.notify_all();
                    }),
                    Arc::new(move |why: String| {
                        lock(&failed).failure = Some(why);
                        failed.1.notify_all();
                    }),
                )
            })?;
        }
        let board = lock(&state);
        let board = state
            .1
            .wait_while(board, |b| {
                b.times.len() < runs as usize && b.failure.is_none()
            })
            .unwrap_or_else(|e| e.into_inner());
        match &board.failure {
            Some(why) => Err(why.clone()),
            None => Ok(board.times.clone()),
        }
    }
}
