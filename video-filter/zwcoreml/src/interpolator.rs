// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 一つの大きさの補間：CoreML のモデル（convert.py）で前・今からフローと重みを求め、
//! Metal で元の大きさへ伸ばして前後をワープし、間へ書く。
//! 受け渡しの方式は metal（IOSurface を直に CoreML と Metal へ）と shm（メインメモリをコピーする）。
//! metal の方式はジョブを二つまで抱え、予測の完了を待たずに要求する（macOS 14 から）。

use std::ffi::{OsString, c_void};
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use block2::{RcBlock, StackBlock};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{AnyThread, ClassType, available, msg_send};
use objc2_core_foundation::{CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_core_ml::{
    MLComputeUnits, MLDictionaryFeatureProvider, MLFeatureProvider, MLModel, MLModelConfiguration,
    MLModelCreatorDefinedKey, MLMultiArray, MLMultiArrayDataType, MLPredictionOptions,
};
use objc2_core_video::{
    CVPixelBuffer, CVPixelBufferCreate, CVPixelBufferCreateWithIOSurface,
    CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetIOSurface,
    CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
    kCVPixelBufferIOSurfacePropertiesKey, kCVPixelBufferMetalCompatibilityKey,
    kCVPixelFormatType_OneComponent16Half, kCVReturnSuccess,
};
use objc2_foundation::{
    NSArray, NSDictionary, NSError, NSInteger, NSNumber, NSString, NSURL, ns_string,
};
use objc2_io_surface::{
    IOSurfaceLockOptions, IOSurfaceRef, kIOSurfaceBytesPerElement, kIOSurfaceBytesPerRow,
    kIOSurfaceHeight, kIOSurfacePixelFormat, kIOSurfaceWidth,
};
use objc2_metal::{
    MTLCommandBuffer, MTLCommandBufferStatus, MTLCommandEncoder, MTLCommandQueue,
    MTLComputeCommandEncoder, MTLComputePipelineState, MTLCreateSystemDefaultDevice, MTLDevice,
    MTLLibrary, MTLPixelFormat, MTLSize, MTLStorageMode, MTLTexture, MTLTextureDescriptor,
    MTLTextureUsage,
};
use zwsdk::video::process::{self as wire, Format, MetalId, Open, Opened, Ran, Run, Share, Way};

use crate::channel::{self, Channel, Mapping};
use crate::report::{MODEL_STATUS, log_line, report_status};

// MTLCreateSystemDefaultDevice は CoreGraphics が要る。
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {}

const MINIMUM_SIDE: u32 = 32;
const ROW_ALIGN: usize = 256;
const PIXEL_F16: u32 = kCVPixelFormatType_OneComponent16Half; // 'L00h'
const TIMED_RUNS: u32 = 300;
/// 同時に抱えるジョブ（ANE は一つずつでも、受け取りと仕上げが重なる）。
const MOST_JOBS: u32 = 2;

const FINISH_SOURCE: &str = include_str!("finish.metal");

type Texture = Retained<ProtocolObject<dyn MTLTexture>>;
type Surface = CFRetained<IOSurfaceRef>;

#[derive(Debug)]
pub struct Options {
    pub models: PathBuf,
    pub name: String,
    pub units: MLComputeUnits,
}

pub fn units_of(text: &str) -> MLComputeUnits {
    match text {
        "gpu" => MLComputeUnits::CPUAndGPU,
        "ane" => MLComputeUnits::CPUAndNeuralEngine,
        "cpu" => MLComputeUnits::CPUOnly,
        _ => MLComputeUnits::All,
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct Params {
    width: u32,
    height: u32,
    low_w: u32,
    low_h: u32,
    inv_k: f32,
}

#[derive(Default)]
struct Picture {
    surface: Option<Surface>,
    texture: Option<Texture>,
    array: Option<Retained<MLMultiArray>>,
    mapping: Option<Mapping>,
    stride: u32,
}

struct Lane {
    flow_buffer: CFRetained<CVPixelBuffer>,
    mask_buffer: CFRetained<CVPixelBuffer>,
    flow_texture: Texture,
    mask_texture: Texture,
    options: Retained<MLPredictionOptions>,
    phase: Retained<MLMultiArray>,
    phase_value: AtomicU32,
}

struct Gpu {
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    pipeline: Retained<ProtocolObject<dyn MTLComputePipelineState>>,
    params: Params,
}

#[derive(Default)]
struct Stats {
    predict_ms: f64,
    finish_ms: f64,
    timed: u32,
}

#[derive(Default)]
struct Flight {
    count: Mutex<u32>,
    done: Condvar,
}

impl Flight {
    fn take_off(&self) {
        *self.count.lock().unwrap_or_else(|e| e.into_inner()) += 1;
    }

    fn land(&self) {
        *self.count.lock().unwrap_or_else(|e| e.into_inner()) -= 1;
        self.done.notify_all();
    }
}

pub type Done = Arc<dyn Fn(Ran)>;
pub type Failed = Arc<dyn Fn(String)>;

pub struct Interpolator {
    way: Way,
    service: String,
    width: u32,
    height: u32,
    jobs: u32,
    asynchronous: bool,
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    model: Retained<MLModel>,
    gpu: Arc<Gpu>,
    low_w: u32,
    low_h: u32,
    backing: bool,
    inputs: Vec<Picture>,
    middles: Vec<Picture>,
    lanes: Vec<Arc<Lane>>,
    flight: Arc<Flight>,
    stats: Arc<Mutex<Stats>>,
}

impl Interpolator {
    pub fn new(options: &Options, open: &Open) -> Result<Self, String> {
        let device = MTLCreateSystemDefaultDevice().ok_or("Metal の GPU が無い")?;
        let (way, service) = choose_way(open, &device)?;
        let (width, height) = (open.width, open.height);
        // 完了を待たない予測（predictionFromFeatures:options:completionHandler:）は macOS 14 から。
        let asynchronous = way == Way::Metal && available!(macos = 14.0);
        let jobs = if asynchronous {
            open.jobs.clamp(1, MOST_JOBS)
        } else {
            1
        };
        log_line(&format!("受け渡しの方式は {}、同時に {jobs} つ", way.name()));

        let library = device
            .newLibraryWithSource_options_error(&NSString::from_str(FINISH_SOURCE), None)
            .map_err(|e| format!("仕上げの Metal を組めない：{}", describe(&e)))?;
        let function = library
            .newFunctionWithName(ns_string!("finish"))
            .ok_or("仕上げの Metal を組めない：finish が無い")?;
        let pipeline = device
            .newComputePipelineStateWithFunction_error(&function)
            .map_err(|e| format!("仕上げの Metal を組めない：{}", describe(&e)))?;
        let queue = device.newCommandQueue().ok_or("Metal のキューを作れない")?;

        let model = load_model(options, width, height)?;
        let (low_w, low_h, padded) = describe_model(&model)?;
        let k = match padded {
            Some(padded) => padded / low_w as i32,
            None => width.div_ceil(low_w) as i32,
        };
        if k < 1 {
            return Err(String::from("モデルのフローの大きさが違う"));
        }
        let gpu = Arc::new(Gpu {
            queue,
            pipeline,
            params: Params {
                width,
                height,
                low_w,
                low_h,
                inv_k: 1.0 / k as f32,
            },
        });
        let mut this = Self {
            way,
            service,
            width,
            height,
            jobs,
            asynchronous,
            device,
            model,
            gpu,
            low_w,
            low_h,
            backing: true,
            inputs: Vec::new(),
            middles: Vec::new(),
            lanes: Vec::new(),
            flight: Arc::default(),
            stats: Arc::default(),
        };
        this.make_lanes(jobs)?;

        // 初めの推論は遅い（ANE へ載せる）ので、ここで一度回しておく。
        report_status(Some(0.8), MODEL_STATUS);
        let blank = blank_array(height, width)?;
        let started = Instant::now();
        let first = this.lanes[0].clone();
        if let Err(why) = this.predict(&first, this.features(&first, &blank, &blank, 0.5)?) {
            log_line(&format!(
                "出力を IOSurface へ直に書かせられない（コピーする）：{why}"
            ));
            this.backing = false;
            for lane in &this.lanes {
                // SAFETY: 空の辞書を置くだけ。
                unsafe { lane.options.setOutputBackings(&NSDictionary::new()) };
            }
            this.predict(&first, this.features(&first, &blank, &blank, 0.5)?)?;
        }
        log_line(&format!(
            "モデル {}-{width}x{height}：フローは {low_w}x{low_h}、初めの推論 {:.0} ms",
            options.name,
            ms_since(started)
        ));
        // ほかのレーンも一度ずつ（出力のバッファごとに初めの一回が遅い）。
        for lane in this.lanes.clone().iter().skip(1) {
            this.predict(lane, this.features(lane, &blank, &blank, 0.5)?)?;
        }
        this.inputs = (0..wire::inputs(jobs))
            .map(|_| Picture::default())
            .collect();
        this.middles = (0..jobs).map(|_| Picture::default()).collect();
        Ok(this)
    }

    pub fn chosen(&self) -> Opened {
        Opened {
            way: self.way as u32,
            format: Format::F16 as u32,
            jobs: self.jobs,
        }
    }

    pub fn service(&self) -> &str {
        &self.service
    }

    pub fn asynchronous(&self) -> bool {
        self.asynchronous
    }

    fn make_lanes(&mut self, count: u32) -> Result<(), String> {
        let (low_w, low_h) = (self.low_w as usize, self.low_h as usize);
        while self.lanes.len() < count as usize {
            let flow_buffer = make_pixel_buffer(low_w, 4 * low_h)?;
            let mask_buffer = make_pixel_buffer(low_w, low_h)?;
            // SAFETY: f16 の一成分の CVPixelBuffer を、幅が同じ形で包む。
            let (flow, mask) = unsafe {
                (
                    MLMultiArray::initWithPixelBuffer_shape(
                        MLMultiArray::alloc(),
                        &flow_buffer,
                        &shape(&[1, 4, low_h, low_w]),
                    ),
                    MLMultiArray::initWithPixelBuffer_shape(
                        MLMultiArray::alloc(),
                        &mask_buffer,
                        &shape(&[1, 1, low_h, low_w]),
                    ),
                )
            };
            let flow_texture = texture_of(&self.device, &surface_of(&flow_buffer)?, false)?;
            let mask_texture = texture_of(&self.device, &surface_of(&mask_buffer)?, false)?;
            // SAFETY: 予測の設定を作り、出力のバッファを名前で渡す。
            let (options, phase) = unsafe {
                let options = MLPredictionOptions::new();
                if self.backing {
                    let backings = NSDictionary::<NSString, AnyObject>::from_slices(
                        &[ns_string!("flow"), ns_string!("mask")],
                        &[as_object(&flow), as_object(&mask)],
                    );
                    options.setOutputBackings(&backings);
                }
                let phase = MLMultiArray::initWithShape_dataType_error(
                    MLMultiArray::alloc(),
                    &shape(&[1, 1, 1, 1]),
                    MLMultiArrayDataType::Float16,
                )
                .map_err(|e| format!("位相の MLMultiArray を作れない：{}", describe(&e)))?;
                (options, phase)
            };
            self.lanes.push(Arc::new(Lane {
                flow_buffer,
                mask_buffer,
                flow_texture,
                mask_texture,
                options,
                phase,
                phase_value: AtomicU32::new((-1.0f32).to_bits()),
            }));
        }
        Ok(())
    }

    pub fn share(&mut self, share: &Share, channel: &mut Channel) -> Result<(), String> {
        let input = match share.what {
            wire::INPUT => true,
            wire::MIDDLE => false,
            _ => return Err(String::from("SHARE の種類が分からない")),
        };
        let (width, height) = (self.width, self.height);
        let pictures = if input {
            &mut self.inputs
        } else {
            &mut self.middles
        };
        let picture = pictures
            .get_mut(share.index as usize)
            .ok_or("SHARE のスロットの番号が大きい")?;
        if (share.stride as usize) < width as usize * 2 {
            return Err(String::from("画像の行の幅が合わない"));
        }
        *picture = Picture {
            stride: share.stride,
            ..Picture::default()
        };
        let surface = if self.way == Way::Metal {
            let port = channel.take_port(share.what, share.index)?;
            let surface = IOSurfaceRef::lookup_from_mach_port(port);
            channel::release_port(port);
            let surface = surface.ok_or("IOSurface を開けない")?;
            if surface.width() != width as usize
                || surface.height() != 3 * height as usize
                || surface.pixel_format() != PIXEL_F16
            {
                return Err(String::from("IOSurface の形が違う"));
            }
            surface
        } else {
            let mapping = Mapping::map(channel.take_fd()?, share.size)?;
            if mapping.size() < share.stride as usize * 3 * height as usize {
                return Err(String::from("共有メモリが小さい"));
            }
            picture.mapping = Some(mapping);
            make_surface(width as usize, 3 * height as usize)?
        };
        picture.texture = Some(texture_of(&self.device, &surface, !input)?);
        if input {
            picture.array = Some(array_of(&surface, 3, height as usize, width as usize)?);
        }
        picture.surface = Some(surface);
        Ok(())
    }

    pub fn use_own_surfaces(&mut self, jobs: u32) -> Result<(), String> {
        let jobs = jobs.clamp(1, if self.asynchronous { MOST_JOBS } else { 1 });
        self.make_lanes(jobs)?;
        self.jobs = jobs;
        let (width, height) = (self.width as usize, self.height as usize);
        let make = |input: bool| -> Result<Picture, String> {
            let surface = make_surface(width, 3 * height)?;
            Ok(Picture {
                stride: surface.bytes_per_row() as u32,
                texture: Some(texture_of(&self.device, &surface, !input)?),
                array: if input {
                    Some(array_of(&surface, 3, height, width)?)
                } else {
                    None
                },
                surface: Some(surface),
                mapping: None,
            })
        };
        self.inputs = (0..wire::inputs(jobs))
            .map(|_| make(true))
            .collect::<Result<_, _>>()?;
        self.middles = (0..jobs).map(|_| make(false)).collect::<Result<_, _>>()?;
        Ok(())
    }

    pub fn jobs(&self) -> u32 {
        self.jobs
    }

    fn features(
        &self,
        lane: &Lane,
        previous: &MLMultiArray,
        current: &MLMultiArray,
        phase: f32,
    ) -> Result<Retained<MLDictionaryFeatureProvider>, String> {
        if lane.phase_value.swap(phase.to_bits(), Ordering::Relaxed) != phase.to_bits() {
            // SAFETY: 一つの値の位相へ書く。
            unsafe {
                lane.phase
                    .setObject_atIndexedSubscript(&NSNumber::new_f32(phase), 0)
            };
        }
        let dictionary = NSDictionary::<NSString, AnyObject>::from_slices(
            &[ns_string!("img0"), ns_string!("img1"), ns_string!("t")],
            &[
                as_object(previous),
                as_object(current),
                as_object(&lane.phase),
            ],
        );
        // SAFETY: 中身は MLMultiArray だけ。
        unsafe {
            MLDictionaryFeatureProvider::initWithDictionary_error(
                MLDictionaryFeatureProvider::alloc(),
                &dictionary,
            )
        }
        .map_err(|e| format!("入力をまとめられない：{}", describe(&e)))
    }

    fn predict(
        &self,
        lane: &Lane,
        inputs: Retained<MLDictionaryFeatureProvider>,
    ) -> Result<(), String> {
        // SAFETY: 入力と出力のバッファはレーンの物。
        let out = unsafe {
            self.model.predictionFromFeatures_options_error(
                ProtocolObject::from_ref(&*inputs),
                &lane.options,
            )
        }
        .map_err(|e| format!("推論できない：{}", describe(&e)))?;
        copy_outputs(lane, &out, self.backing, self.low_w, self.low_h)
    }

    fn check_run(&self, run: &Run) -> Result<(), String> {
        let ready = |pictures: &[Picture], index: u32| {
            pictures
                .get(index as usize)
                .ok_or("RUN のスロットの番号が大きい")?
                .surface
                .as_ref()
                .map(|_| ())
                .ok_or("SHARE が足りない")
        };
        ready(&self.inputs, run.previous)?;
        ready(&self.inputs, run.current)?;
        ready(&self.middles, run.middle)?;
        Ok(())
    }

    pub fn run(&mut self, run: &Run) -> Result<Ran, String> {
        self.check_run(run)?;
        let started = Instant::now();
        let bytes = self.width as usize * 2;
        let rows = 3 * self.height as usize;
        if self.way == Way::Shm {
            // メインメモリから手元の IOSurface へ（一つずつ処理するので、同じスロットを読んでいるジョブは無い）。
            for index in [run.previous, run.current] {
                let picture = &self.inputs[index as usize];
                let (Some(surface), Some(mapping)) = (&picture.surface, &picture.mapping) else {
                    return Err(String::from("SHARE が足りない"));
                };
                // SAFETY: ロックした IOSurface とマップしたメインメモリの、行の幅の内をコピーする。
                unsafe {
                    surface.lock(IOSurfaceLockOptions::empty(), std::ptr::null_mut());
                    copy_rows(
                        surface.base_address().as_ptr().cast(),
                        surface.bytes_per_row(),
                        mapping.data(),
                        picture.stride as usize,
                        rows,
                        bytes,
                    );
                    surface.unlock(IOSurfaceLockOptions::empty(), std::ptr::null_mut());
                }
            }
        }
        let lane = self.lanes[run.middle as usize].clone();
        let previous = &self.inputs[run.previous as usize];
        let current = &self.inputs[run.current as usize];
        let middle = &self.middles[run.middle as usize];
        let (Some(previous_array), Some(current_array)) = (&previous.array, &current.array) else {
            return Err(String::from("SHARE が足りない"));
        };
        self.predict(
            &lane,
            self.features(&lane, previous_array, current_array, run.phase)?,
        )?;
        let predicted = ms_since(started);
        let commands = self.gpu.finish(
            &lane,
            textures(previous)?,
            textures(current)?,
            textures(middle)?,
        )?;
        commands.commit();
        commands.waitUntilCompleted();
        if commands.status() != MTLCommandBufferStatus::Completed {
            return Err(String::from("仕上げの Metal が終わらない"));
        }
        if self.way == Way::Shm {
            let (Some(surface), Some(mapping)) = (&middle.surface, &middle.mapping) else {
                return Err(String::from("SHARE が足りない"));
            };
            // SAFETY: ロックした IOSurface から、マップしたメインメモリの行の幅の内へコピーする。
            unsafe {
                surface.lock(IOSurfaceLockOptions::ReadOnly, std::ptr::null_mut());
                copy_rows(
                    mapping.data(),
                    middle.stride as usize,
                    surface.base_address().as_ptr().cast(),
                    surface.bytes_per_row(),
                    rows,
                    bytes,
                );
                surface.unlock(IOSurfaceLockOptions::ReadOnly, std::ptr::null_mut());
            }
        }
        let total = ms_since(started);
        record(&self.stats, predicted, total);
        Ok(Ran {
            value: run.value,
            gpu_ms: total as f32,
        })
    }

    /// 要求して戻る。書き終えたら done（失敗なら failed）を CoreML か Metal のスレッドから呼ぶ。
    pub fn run_async(&self, run: &Run, done: Done, failed: Failed) -> Result<(), String> {
        if !available!(macos = 14.0) {
            return Err(String::from("完了を待たない予測が無い（macOS 14 から）"));
        }
        self.check_run(run)?;
        let started = Instant::now();
        let lane = self.lanes[run.middle as usize].clone();
        let previous = &self.inputs[run.previous as usize];
        let current = &self.inputs[run.current as usize];
        let (Some(previous_array), Some(current_array)) = (&previous.array, &current.array) else {
            return Err(String::from("SHARE が足りない"));
        };
        let inputs = self.features(&lane, previous_array, current_array, run.phase)?;
        let textures = [
            textures(previous)?.clone(),
            textures(current)?.clone(),
            textures(&self.middles[run.middle as usize])?.clone(),
        ];
        let (gpu, stats, flight) = (self.gpu.clone(), self.stats.clone(), self.flight.clone());
        let (backing, low_w, low_h, value) = (self.backing, self.low_w, self.low_h, run.value);
        let options = lane.options.clone();
        flight.take_off();
        let handler = RcBlock::new(
            move |out: *mut ProtocolObject<dyn MLFeatureProvider>, error: *mut NSError| {
                // SAFETY: CoreML が渡す出力と失敗（どちらかが有る）。
                let Some(out) = (unsafe { out.as_ref() }) else {
                    let why = unsafe { error.as_ref() }.map_or_else(|| String::from("?"), describe);
                    failed(format!("推論できない：{why}"));
                    flight.land();
                    return;
                };
                let predicted = ms_since(started);
                let commands = copy_outputs(&lane, out, backing, low_w, low_h)
                    .and_then(|()| gpu.finish(&lane, &textures[0], &textures[1], &textures[2]));
                let commands = match commands {
                    Ok(commands) => commands,
                    Err(why) => {
                        failed(why);
                        flight.land();
                        return;
                    }
                };
                let (done, failed, stats, flight) =
                    (done.clone(), failed.clone(), stats.clone(), flight.clone());
                let completed = RcBlock::new(
                    move |finished: NonNull<ProtocolObject<dyn MTLCommandBuffer>>| {
                        // SAFETY: Metal が渡す、完了したコマンドバッファ。
                        let finished = unsafe { finished.as_ref() };
                        if finished.status() == MTLCommandBufferStatus::Completed {
                            let total = ms_since(started);
                            record(&stats, predicted, total);
                            done(Ran {
                                value,
                                gpu_ms: total as f32,
                            });
                        } else {
                            failed(String::from("仕上げの Metal が終わらない"));
                        }
                        flight.land();
                    },
                );
                // SAFETY: Metal が手元のコピーを持つ。
                unsafe {
                    commands.addCompletedHandler(RcBlock::as_ptr(&completed));
                }
                commands.commit();
            },
        );
        // SAFETY: 入力と出力のバッファはレーンの物。CoreML が手元のコピーを持つ。
        unsafe {
            self.model.predictionFromFeatures_options_completionHandler(
                ProtocolObject::from_ref(&*inputs),
                &options,
                &handler,
            );
        }
        Ok(())
    }
}

impl Drop for Interpolator {
    fn drop(&mut self) {
        // 要求したジョブが完了してから破棄する（完了しなければ 2 秒でタイムアウト）。
        let count = self.flight.count.lock().unwrap_or_else(|e| e.into_inner());
        let _ = self
            .flight
            .done
            .wait_timeout_while(count, Duration::from_secs(2), |count| *count > 0);
    }
}

impl Gpu {
    fn finish(
        &self,
        lane: &Lane,
        previous: &Texture,
        current: &Texture,
        middle: &Texture,
    ) -> Result<Retained<ProtocolObject<dyn MTLCommandBuffer>>, String> {
        let p = self.params;
        let commands = self
            .queue
            .commandBuffer()
            .ok_or("Metal のコマンドバッファを作れない")?;
        let encoder = commands
            .computeCommandEncoder()
            .ok_or("Metal のエンコーダーを作れない")?;
        encoder.setComputePipelineState(&self.pipeline);
        // SAFETY: finish.metal の引数の並びの通りに渡す。
        unsafe {
            encoder.setTexture_atIndex(Some(&**previous), 0);
            encoder.setTexture_atIndex(Some(&**current), 1);
            encoder.setTexture_atIndex(Some(&**middle), 2);
            encoder.setTexture_atIndex(Some(&lane.flow_texture), 3);
            encoder.setTexture_atIndex(Some(&lane.mask_texture), 4);
            encoder.setBytes_length_atIndex(NonNull::from(&p).cast(), size_of::<Params>(), 0);
        }
        encoder.dispatchThreads_threadsPerThreadgroup(
            MTLSize {
                width: p.width as usize,
                height: p.height as usize,
                depth: 1,
            },
            MTLSize {
                width: 16,
                height: 16,
                depth: 1,
            },
        );
        encoder.endEncoding();
        Ok(commands)
    }
}

fn textures(picture: &Picture) -> Result<&Texture, String> {
    picture
        .texture
        .as_ref()
        .ok_or_else(|| String::from("SHARE が足りない"))
}

fn record(stats: &Mutex<Stats>, predicted: f64, total: f64) {
    let mut stats = stats.lock().unwrap_or_else(|e| e.into_inner());
    stats.predict_ms += predicted;
    stats.finish_ms += total - predicted;
    stats.timed += 1;
    if stats.timed == TIMED_RUNS {
        let n = f64::from(stats.timed);
        log_line(&format!(
            "{} 回の平均：推論 {:.1} ms、仕上げ {:.1} ms（要求してから書き終えるまで）",
            stats.timed,
            stats.predict_ms / n,
            stats.finish_ms / n
        ));
        *stats = Stats::default();
    }
}

fn describe(error: &NSError) -> String {
    error.localizedDescription().to_string()
}

fn ms_since(from: Instant) -> f64 {
    from.elapsed().as_secs_f64() * 1000.0
}

fn as_object(object: &AnyObject) -> &AnyObject {
    object
}

fn shape(dims: &[usize]) -> Retained<NSArray<NSNumber>> {
    let numbers: Vec<_> = dims.iter().map(|&d| NSNumber::new_usize(d)).collect();
    NSArray::from_retained_slice(&numbers)
}

fn choose_way(
    open: &Open,
    device: &ProtocolObject<dyn MTLDevice>,
) -> Result<(Way, String), String> {
    if open.width < MINIMUM_SIDE || open.height < MINIMUM_SIDE {
        return Err(String::from("画像が小さすぎる"));
    }
    if !open.formats.contains(&(Format::F16 as u32)) {
        return Err(String::from("f16 の画像を出さない本体"));
    }
    for offer in &open.offers {
        match Way::of(offer.way) {
            Some(Way::Metal) => {
                let id = MetalId::decode(&offer.id)?;
                let ours = device.registryID();
                if id.registry_id != ours {
                    log_line(&format!(
                        "metal の方式：レンダリングする GPU（{:x}）がこちらの GPU（{ours:x}）と違う",
                        id.registry_id
                    ));
                }
                return Ok((Way::Metal, id.service));
            }
            Some(Way::Shm) => return Ok((Way::Shm, String::new())),
            _ => {}
        }
    }
    Err(String::from("使える受け渡しの方式が無い（metal か shm）"))
}

fn describe_model(model: &MLModel) -> Result<(u32, u32, Option<i32>), String> {
    let description = unsafe { model.modelDescription() };
    let shape = unsafe { description.outputDescriptionsByName() }
        .objectForKey(ns_string!("flow"))
        .and_then(|flow| unsafe { flow.multiArrayConstraint() })
        .map(|constraint| unsafe { constraint.shape() })
        .map(|shape| shape.iter().map(|n| n.as_u32()).collect::<Vec<_>>())
        .filter(|shape| shape.len() == 4 && shape[1] == 4)
        .ok_or("モデルの出力（flow）の形が違う")?;
    let padded = unsafe { MLModelCreatorDefinedKey }
        .and_then(|key| unsafe { description.metadata() }.objectForKey(key))
        .filter(|creator| {
            let is_dictionary: bool = unsafe {
                msg_send![&**creator, isKindOfClass: <NSDictionary as ClassType>::class()]
            };
            is_dictionary
        })
        .and_then(|creator| {
            let creator: Retained<NSDictionary<NSString, AnyObject>> =
                unsafe { Retained::cast_unchecked(creator) };
            creator.objectForKey(ns_string!("padded_width"))
        })
        .and_then(|padded| padded.downcast::<NSString>().ok())
        .map(|padded| padded.intValue());
    Ok((shape[3], shape[2], padded))
}

fn compiled_paths(options: &Options, width: u32, height: u32) -> (PathBuf, PathBuf) {
    let base = options
        .models
        .join(format!("{}-{width}x{height}", options.name));
    let with = |extension: &str| {
        let mut path = OsString::from(base.as_os_str());
        path.push(extension);
        PathBuf::from(path)
    };
    (with(".mlpackage"), with(".mlmodelc"))
}

/// フォルダーの `<name>-<幅>x<高さ>.mlpackage` を（無いか古ければ）`.mlmodelc` へ変換して読む。
fn load_model(options: &Options, width: u32, height: u32) -> Result<Retained<MLModel>, String> {
    let (package, mut compiled) = compiled_paths(options, width, height);
    let has_package = package.exists();
    let mut has_compiled = compiled.exists();
    if !has_package && !has_compiled {
        return Err(format!(
            "モデルが無い：{}（zwcoreml の README の手順で作る）",
            package.display()
        ));
    }
    if has_package && has_compiled {
        let modified = |path: &Path| std::fs::metadata(path).and_then(|m| m.modified()).ok();
        if let (Some(made), Some(copied)) = (
            modified(&package.join("Manifest.json")),
            modified(&compiled),
        ) && made > copied
        {
            has_compiled = false;
        }
    }
    if !has_compiled {
        report_status(Some(0.1), MODEL_STATUS);
        let url = file_url(&package);
        #[allow(deprecated)]
        // SAFETY: mlpackage を一時フォルダーへ変換させる。
        let temporary = unsafe { MLModel::compileModelAtURL_error(&url) }
            .map_err(|e| format!("モデルを CoreML の形にできない：{}", describe(&e)))?;
        let temporary = temporary
            .path()
            .map(|path| PathBuf::from(path.to_string()))
            .ok_or("変換したモデルの場所が分からない")?;
        let _ = std::fs::remove_dir_all(&compiled);
        if let Err(e) =
            std::fs::rename(&temporary, &compiled).or_else(|_| copy_dir(&temporary, &compiled))
        {
            log_line(&format!(
                "変換したモデルをフォルダーへ移せない（一時フォルダーから読む）：{e}"
            ));
            compiled = temporary;
        }
    }
    report_status(Some(0.4), MODEL_STATUS);
    // SAFETY: 変換したモデルを、計算に使うデバイスを決めて読む。
    unsafe {
        let configuration = MLModelConfiguration::new();
        configuration.setComputeUnits(options.units);
        MLModel::modelWithContentsOfURL_configuration_error(&file_url(&compiled), &configuration)
    }
    .map_err(|e| format!("モデルを読めない：{}", describe(&e)))
}

fn file_url(path: &Path) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn make_surface(width: usize, height: usize) -> Result<Surface, String> {
    let row = (width * 2).next_multiple_of(ROW_ALIGN);
    let values = [width, height, 2, row, PIXEL_F16 as usize].map(|v| CFNumber::new_i64(v as i64));
    // SAFETY: IOSurface のキー（外部の静的な CFString）を読む。
    let keys: [&CFString; 5] = unsafe {
        [
            kIOSurfaceWidth,
            kIOSurfaceHeight,
            kIOSurfaceBytesPerElement,
            kIOSurfaceBytesPerRow,
            kIOSurfacePixelFormat,
        ]
    };
    let values: Vec<&CFNumber> = values.iter().map(|v| &**v).collect();
    let properties = CFDictionary::<CFString, CFNumber>::from_slices(&keys, &values);
    // SAFETY: キーと値は IOSurface が決めた型。
    unsafe { IOSurfaceRef::new(properties.as_opaque()) }
        .ok_or_else(|| String::from("IOSurface を作れない"))
}

fn make_pixel_buffer(width: usize, height: usize) -> Result<CFRetained<CVPixelBuffer>, String> {
    let empty = CFDictionary::<CFString, CFType>::empty();
    // SAFETY: CoreVideo のキー（外部の静的な CFString）を読む。
    let keys: [&CFString; 2] = unsafe {
        [
            kCVPixelBufferIOSurfacePropertiesKey,
            kCVPixelBufferMetalCompatibilityKey,
        ]
    };
    let values: [&CFType; 2] = [&empty, CFBoolean::new(true)];
    let attributes = CFDictionary::<CFString, CFType>::from_slices(&keys, &values);
    let mut buffer: *mut CVPixelBuffer = std::ptr::null_mut();
    // SAFETY: 出力はローカル変数。作れたら所有権はこちら。
    unsafe {
        if CVPixelBufferCreate(
            None,
            width,
            height,
            PIXEL_F16,
            Some(attributes.as_opaque()),
            NonNull::from(&mut buffer),
        ) != kCVReturnSuccess
        {
            return Err(String::from("CVPixelBuffer を作れない"));
        }
        NonNull::new(buffer)
            .map(|buffer| CFRetained::from_raw(buffer))
            .ok_or_else(|| String::from("CVPixelBuffer を作れない"))
    }
}

fn surface_of(buffer: &CVPixelBuffer) -> Result<Surface, String> {
    CVPixelBufferGetIOSurface(Some(buffer))
        .ok_or_else(|| String::from("CVPixelBuffer に IOSurface が無い"))
}

fn texture_of(
    device: &ProtocolObject<dyn MTLDevice>,
    surface: &Surface,
    write: bool,
) -> Result<Texture, String> {
    // SAFETY: IOSurface の大きさの R16Float のテクスチャ。
    unsafe {
        let descriptor =
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::R16Float,
                surface.width(),
                surface.height(),
                false,
            );
        descriptor.setUsage(if write {
            MTLTextureUsage::ShaderWrite
        } else {
            MTLTextureUsage::ShaderRead
        });
        descriptor.setStorageMode(MTLStorageMode::Shared);
        device.newTextureWithDescriptor_iosurface_plane(&descriptor, surface, 0)
    }
    .ok_or_else(|| String::from("IOSurface を Metal のテクスチャにできない"))
}

fn array_of(
    surface: &Surface,
    planes: usize,
    height: usize,
    width: usize,
) -> Result<Retained<MLMultiArray>, String> {
    let mut buffer: *mut CVPixelBuffer = std::ptr::null_mut();
    // SAFETY: 出力はローカル変数。包んだ CVPixelBuffer は MLMultiArray が持つ。
    unsafe {
        if CVPixelBufferCreateWithIOSurface(None, surface, None, NonNull::from(&mut buffer))
            != kCVReturnSuccess
        {
            return Err(String::from("IOSurface を CVPixelBuffer に包めない"));
        }
        let buffer: CFRetained<CVPixelBuffer> = CFRetained::from_raw(
            NonNull::new(buffer).ok_or("IOSurface を CVPixelBuffer に包めない")?,
        );
        Ok(MLMultiArray::initWithPixelBuffer_shape(
            MLMultiArray::alloc(),
            &buffer,
            &shape(&[1, planes, height, width]),
        ))
    }
}

fn blank_array(height: u32, width: u32) -> Result<Retained<MLMultiArray>, String> {
    // SAFETY: 形を決めて作り、中身を 0 で埋める。
    unsafe {
        let blank = MLMultiArray::initWithShape_dataType_error(
            MLMultiArray::alloc(),
            &shape(&[1, 3, height as usize, width as usize]),
            MLMultiArrayDataType::Float16,
        )
        .map_err(|e| format!("MLMultiArray を作れない：{}", describe(&e)))?;
        let zero = StackBlock::new(
            |bytes: NonNull<c_void>, size: NSInteger, _strides: NonNull<NSArray<NSNumber>>| {
                std::ptr::write_bytes(bytes.as_ptr().cast::<u8>(), 0, size as usize);
            },
        );
        blank.getMutableBytesWithHandler(&zero);
        Ok(blank)
    }
}

/// 行ごとにコピーする（コピーするのは各行の先頭の bytes）。
///
/// # Safety
/// 両方が rows 行分読み書きできること。
unsafe fn copy_rows(
    to: *mut u8,
    to_stride: usize,
    from: *const u8,
    from_stride: usize,
    rows: usize,
    bytes: usize,
) {
    for y in 0..rows {
        // SAFETY: 呼ぶ側が rows 行分を約束する。
        unsafe {
            std::ptr::copy_nonoverlapping(from.add(y * from_stride), to.add(y * to_stride), bytes);
        }
    }
}

fn copy_outputs(
    lane: &Lane,
    out: &ProtocolObject<dyn MLFeatureProvider>,
    backing: bool,
    low_w: u32,
    low_h: u32,
) -> Result<(), String> {
    if backing {
        return Ok(());
    }
    for (name, buffer, planes) in [
        (ns_string!("flow"), &lane.flow_buffer, 4),
        (ns_string!("mask"), &lane.mask_buffer, 1),
    ] {
        // SAFETY: 予測の出力を名前で取り出す。
        let array = unsafe { out.featureValueForName(name) }
            .and_then(|value| unsafe { value.multiArrayValue() })
            .ok_or("予測の出力が無い")?;
        copy_array(&array, buffer, planes, low_h as usize, low_w as usize);
    }
    Ok(())
}

fn copy_array(from: &MLMultiArray, to: &CVPixelBuffer, planes: usize, height: usize, width: usize) {
    // SAFETY: ロックした CVPixelBuffer へ、MLMultiArray のストライドの通りにコピーする。
    unsafe {
        CVPixelBufferLockBaseAddress(to, CVPixelBufferLockFlags(0));
        let base = CVPixelBufferGetBaseAddress(to).cast::<u8>();
        let row = CVPixelBufferGetBytesPerRow(to);
        let strides = from.strides();
        let plane_stride = strides.objectAtIndex(1).as_usize();
        let row_stride = strides.objectAtIndex(2).as_usize();
        let copy = StackBlock::new(|bytes: NonNull<c_void>, _size: NSInteger| {
            let data = bytes.as_ptr().cast::<u16>();
            for c in 0..planes {
                for y in 0..height {
                    std::ptr::copy_nonoverlapping(
                        data.add(c * plane_stride + y * row_stride),
                        base.add((c * height + y) * row).cast::<u16>(),
                        width,
                    );
                }
            }
        });
        from.getBytesWithHandler(&copy);
        CVPixelBufferUnlockBaseAddress(to, CVPixelBufferLockFlags(0));
    }
}

pub fn bench_run(i: u32, jobs: u32) -> Run {
    Run {
        value: u64::from(i) + 1,
        phase: 0.5,
        previous: i % (jobs + 1),
        current: (i + 1) % (jobs + 1),
        middle: i % jobs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bench_jobs_follow_the_slots() {
        let runs: Vec<_> = (0..4).map(|i| bench_run(i, 2)).collect();
        for pair in runs.windows(2) {
            assert_eq!(pair[1].previous, pair[0].current);
            assert_ne!(pair[1].middle, pair[0].middle);
        }
        assert_eq!(bench_run(0, 1).middle, 0);
        assert_eq!(bench_run(1, 1).previous, 1);
    }
}
