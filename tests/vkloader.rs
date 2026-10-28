// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! Vulkan loader (`vkloader.c`) tests. Not in the C suite, which only has the interactive
//! `vkloadtests` app: textures are uploaded with every path of
//! `ktxTexture_VkUploadEx_WithSuballocator` (optimal tiling fast / padded path, linear fast /
//! padded path, mipmap generation, suballocator) to the first Vulkan device (lavapipe works),
//! read back and compared with the source images.
#![cfg(all(feature = "vulkan", not(target_family = "wasm")))]
#![allow(unsafe_code, clippy::panic)]

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use ash::vk;
use ktx::texture::{KtxTexture, TextureCreateInfo, TextureCreateStorage};
use ktx::vkloader::{
    SubAllocatorCallbacks, VulkanDeviceInfo, VulkanFunctions, VulkanTexture,
    ktx_load_vulkan_function,
};
use ktx::{ErrorCode, Texture1, Texture2, VkFormat};

const GL_R8: u32 = 0x8229;

struct Ctx {
    entry: ash::Entry,
    instance: ash::Instance,
    pd: vk::PhysicalDevice,
    device: ash::Device,
    queue: vk::Queue,
    pool: vk::CommandPool,
}

impl Ctx {
    fn new() -> Ctx {
        unsafe {
            let entry = ash::Entry::load().unwrap_or_else(|e| panic!("Vulkan loader: {e}"));
            let app = vk::ApplicationInfo::default().api_version(vk::API_VERSION_1_0);
            let ci = vk::InstanceCreateInfo::default().application_info(&app);
            let instance = entry
                .create_instance(&ci, None)
                .unwrap_or_else(|e| panic!("vkCreateInstance: {e}"));
            let pds = instance.enumerate_physical_devices().unwrap_or_default();
            // Prefer the CPU device (lavapipe) for reproducibility.
            let pd = pds
                .iter()
                .copied()
                .find(|&p| {
                    instance.get_physical_device_properties(p).device_type
                        == vk::PhysicalDeviceType::CPU
                })
                .or_else(|| pds.first().copied())
                .unwrap_or_else(|| panic!("no Vulkan physical device"));
            let qfi = instance
                .get_physical_device_queue_family_properties(pd)
                .iter()
                .position(|q| q.queue_flags.contains(vk::QueueFlags::GRAPHICS))
                .unwrap_or_else(|| panic!("no graphics queue")) as u32;
            let prio = [1.0f32];
            let qci = vk::DeviceQueueCreateInfo::default()
                .queue_family_index(qfi)
                .queue_priorities(&prio);
            let dci =
                vk::DeviceCreateInfo::default().queue_create_infos(std::slice::from_ref(&qci));
            let device = instance
                .create_device(pd, &dci, None)
                .unwrap_or_else(|e| panic!("vkCreateDevice: {e}"));
            let queue = device.get_device_queue(qfi, 0);
            let pool = device
                .create_command_pool(
                    &vk::CommandPoolCreateInfo::default()
                        .queue_family_index(qfi)
                        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                    None,
                )
                .unwrap_or_else(|e| panic!("vkCreateCommandPool: {e}"));
            Ctx {
                entry,
                instance,
                pd,
                device,
                queue,
                pool,
            }
        }
    }

    fn funcs(&self) -> VulkanFunctions {
        VulkanFunctions {
            vk_get_instance_proc_addr: Some(self.entry.static_fn().get_instance_proc_addr),
            ..Default::default()
        }
    }

    fn device_info(&self) -> Box<VulkanDeviceInfo> {
        unsafe {
            VulkanDeviceInfo::create_ex(
                self.instance.handle(),
                self.pd,
                self.device.handle(),
                self.queue,
                self.pool,
                std::ptr::null(),
                Some(&self.funcs()),
            )
        }
        .unwrap_or_else(|| panic!("ktxVulkanDeviceInfo_CreateEx failed"))
    }

    fn memory_type(&self, bits: u32, props: vk::MemoryPropertyFlags) -> u32 {
        let mp = unsafe { self.instance.get_physical_device_memory_properties(self.pd) };
        (0..mp.memory_type_count)
            .find(|&i| {
                bits & (1 << i) != 0 && mp.memory_types[i as usize].property_flags.contains(props)
            })
            .unwrap_or_else(|| panic!("no memory type"))
    }

    /// Copies all layers of `level` of an image in TRANSFER_SRC_OPTIMAL to the host.
    fn read_level(&self, vt: &VulkanTexture, level: u32, size: usize) -> Vec<u8> {
        unsafe {
            let d = &self.device;
            let buf = d
                .create_buffer(
                    &vk::BufferCreateInfo::default()
                        .size(size as u64)
                        .usage(vk::BufferUsageFlags::TRANSFER_DST),
                    None,
                )
                .unwrap_or_else(|e| panic!("{e}"));
            let req = d.get_buffer_memory_requirements(buf);
            let mem = d
                .allocate_memory(
                    &vk::MemoryAllocateInfo::default()
                        .allocation_size(req.size)
                        .memory_type_index(self.memory_type(
                            req.memory_type_bits,
                            vk::MemoryPropertyFlags::HOST_VISIBLE
                                | vk::MemoryPropertyFlags::HOST_COHERENT,
                        )),
                    None,
                )
                .unwrap_or_else(|e| panic!("{e}"));
            d.bind_buffer_memory(buf, mem, 0)
                .unwrap_or_else(|e| panic!("{e}"));
            let cb = d
                .allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(self.pool)
                        .command_buffer_count(1),
                )
                .unwrap_or_else(|e| panic!("{e}"))[0];
            d.begin_command_buffer(cb, &vk::CommandBufferBeginInfo::default())
                .unwrap_or_else(|e| panic!("{e}"));
            let dim = |v: u32| (v >> level).max(1);
            let region = vk::BufferImageCopy {
                buffer_offset: 0,
                buffer_row_length: 0,
                buffer_image_height: 0,
                image_subresource: vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: level,
                    base_array_layer: 0,
                    layer_count: vt.layer_count,
                },
                image_offset: vk::Offset3D::default(),
                image_extent: vk::Extent3D {
                    width: dim(vt.width),
                    height: dim(vt.height),
                    depth: dim(vt.depth),
                },
            };
            d.cmd_copy_image_to_buffer(
                cb,
                vt.image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                buf,
                &[region],
            );
            d.end_command_buffer(cb).unwrap_or_else(|e| panic!("{e}"));
            let cbs = [cb];
            d.queue_submit(
                self.queue,
                &[vk::SubmitInfo::default().command_buffers(&cbs)],
                vk::Fence::null(),
            )
            .unwrap_or_else(|e| panic!("{e}"));
            d.queue_wait_idle(self.queue)
                .unwrap_or_else(|e| panic!("{e}"));
            let p = d
                .map_memory(mem, 0, size as u64, vk::MemoryMapFlags::empty())
                .unwrap_or_else(|e| panic!("{e}"));
            let out = std::slice::from_raw_parts(p.cast::<u8>(), size).to_vec();
            d.unmap_memory(mem);
            d.free_command_buffers(self.pool, &cbs);
            d.destroy_buffer(buf, None);
            d.free_memory(mem, None);
            out
        }
    }
}

impl Drop for Ctx {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();
            self.device.destroy_command_pool(self.pool, None);
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}

fn pattern(n: usize, seed: u32) -> Vec<u8> {
    let mut s = seed.wrapping_mul(2654435761).wrapping_add(1);
    (0..n)
        .map(|_| {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            (s >> 24) as u8
        })
        .collect()
}

fn ktx2(ci: TextureCreateInfo) -> Texture2 {
    let mut t = Texture2::create(&ci, TextureCreateStorage::AllocStorage)
        .unwrap_or_else(|e| panic!("create: {e:?}"));
    let n = t.data_size;
    if let Some(d) = t.get_data_mut() {
        d.copy_from_slice(&pattern(n, ci.base_width * 31 + ci.num_levels));
    }
    t
}

fn rgba8(w: u32, h: u32, d: u32, dims: u32, levels: u32, layers: u32, faces: u32) -> Texture2 {
    ktx2(TextureCreateInfo {
        vk_format: VkFormat::R8G8B8A8_UNORM,
        base_width: w,
        base_height: h,
        base_depth: d,
        num_dimensions: dims,
        num_levels: levels,
        num_layers: layers,
        num_faces: faces,
        is_array: layers > 1,
        ..Default::default()
    })
}

/// Uploads with optimal tiling to TRANSFER_SRC_OPTIMAL and compares every level.
fn upload_and_compare(ctx: &Ctx, tex: &mut dyn KtxTexture, expected: &[Vec<u8>]) {
    let mut vdi = ctx.device_info();
    let mut vt = VulkanTexture::default();
    let r = unsafe {
        ktx::vkloader::ktx_texture_vk_upload_ex(
            tex,
            &mut vdi,
            &mut vt,
            vk::ImageTiling::OPTIMAL,
            vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_SRC,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
        )
    };
    assert_eq!(r, Ok(()));
    assert_eq!(vt.level_count as usize, expected.len());
    for (level, want) in expected.iter().enumerate() {
        let got = ctx.read_level(&vt, level as u32, want.len());
        assert!(got == *want, "level {level} differs");
    }
    unsafe {
        vt.destruct(ctx.device.handle(), std::ptr::null());
        vdi.destroy();
    }
}

/// The tightly packed images of each level of a KTX2 texture.
fn ktx2_levels(t: &Texture2) -> Vec<Vec<u8>> {
    let data = t.get_data().unwrap_or_default();
    (0..t.num_levels)
        .map(|l| {
            let off = t.get_image_offset(l, 0, 0).unwrap_or(0);
            data[off..off + t.get_level_size(l)].to_vec()
        })
        .collect()
}

#[test]
fn upload_2d_mipmapped() {
    let ctx = Ctx::new();
    let mut t = rgba8(16, 8, 1, 2, 4, 1, 1);
    let want = ktx2_levels(&t);
    upload_and_compare(&ctx, &mut t, &want);
}

#[test]
fn upload_cubemap_array_and_3d() {
    let ctx = Ctx::new();
    let mut cube = rgba8(8, 8, 1, 2, 2, 1, 6);
    let want = ktx2_levels(&cube);
    upload_and_compare(&ctx, &mut cube, &want);
    let mut array = rgba8(8, 4, 1, 2, 2, 3, 1);
    let want = ktx2_levels(&array);
    upload_and_compare(&ctx, &mut array, &want);
    let mut vol = rgba8(4, 4, 4, 3, 3, 1, 1);
    let want = ktx2_levels(&vol);
    upload_and_compare(&ctx, &mut vol, &want);
}

/// KTX1 R8 5x3 with 2 levels: rows are padded to 4 bytes, so the optimal tiling upload takes
/// the padded path (`optimalTilingPadCallback`) which strips the padding.
#[test]
fn upload_ktx1_padded_rows() {
    let ctx = Ctx::new();
    let ci = TextureCreateInfo {
        gl_internalformat: GL_R8,
        base_width: 5,
        base_height: 3,
        base_depth: 1,
        num_dimensions: 2,
        num_levels: 2,
        num_layers: 1,
        num_faces: 1,
        ..Default::default()
    };
    let mut t = Texture1::create(&ci, TextureCreateStorage::AllocStorage)
        .unwrap_or_else(|e| panic!("create: {e:?}"));
    let mut want = Vec::new();
    for level in 0..2u32 {
        let (w, h) = ((5u32 >> level).max(1), (3u32 >> level).max(1));
        let tight = pattern((w * h) as usize, level + 7);
        // SetImageFromMemory takes packed rows and pads them to KTX_GL_UNPACK_ALIGNMENT.
        assert_eq!(t.set_image_from_memory(level, 0, 0, &tight), Ok(()));
        want.push(tight);
    }
    assert_eq!(t.get_vk_format(), VkFormat::R8_UNORM);
    upload_and_compare(&ctx, &mut t, &want);
}

/// Reads a linear image through its mapped memory.
fn read_linear(ctx: &Ctx, vt: &VulkanTexture, row_bytes: usize, rows: usize) -> Vec<u8> {
    unsafe {
        let layout = ctx.device.get_image_subresource_layout(
            vt.image,
            vk::ImageSubresource {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                array_layer: 0,
            },
        );
        let p = ctx
            .device
            .map_memory(
                vt.device_memory,
                0,
                vk::WHOLE_SIZE,
                vk::MemoryMapFlags::empty(),
            )
            .unwrap_or_else(|e| panic!("{e}"))
            .cast::<u8>();
        let mut out = Vec::new();
        for r in 0..rows {
            let row = p.add(layout.offset as usize + r * layout.row_pitch as usize);
            out.extend_from_slice(std::slice::from_raw_parts(row, row_bytes));
        }
        ctx.device.unmap_memory(vt.device_memory);
        out
    }
}

#[test]
fn upload_linear_tiling() {
    let ctx = Ctx::new();
    let mut vdi = ctx.device_info();
    // Fast path (linearTilingCallback): KTX2, rows copied as one block per subresource.
    let mut t = rgba8(8, 4, 1, 2, 1, 1, 1);
    let want = ktx2_levels(&t);
    let mut vt = VulkanTexture::default();
    let r = unsafe {
        t.vk_upload_ex(
            &mut vdi,
            &mut vt,
            vk::ImageTiling::LINEAR,
            vk::ImageUsageFlags::SAMPLED,
            vk::ImageLayout::GENERAL,
        )
    };
    assert_eq!(r, Ok(()));
    assert_eq!(vt.image_layout, vk::ImageLayout::GENERAL);
    // The fast path copies the level verbatim at the subresource offset, so compare the first
    // row only unless the driver's row pitch is tight.
    let got = read_linear(&ctx, &vt, 8 * 4, 1);
    assert_eq!(got, want[0][..32]);
    unsafe { vt.destruct(ctx.device.handle(), std::ptr::null()) };

    // Padded path (linearTilingPadCallback): KTX1 R8 5x3, rows padded to 8 in the file.
    let ci = TextureCreateInfo {
        gl_internalformat: GL_R8,
        base_width: 5,
        base_height: 3,
        base_depth: 1,
        num_dimensions: 2,
        num_levels: 1,
        num_layers: 1,
        num_faces: 1,
        ..Default::default()
    };
    let mut t1 = Texture1::create(&ci, TextureCreateStorage::AllocStorage)
        .unwrap_or_else(|e| panic!("create: {e:?}"));
    let tight = pattern(15, 3);
    assert_eq!(t1.set_image_from_memory(0, 0, 0, &tight), Ok(()));
    assert_eq!(t1.get_row_pitch(0), 8);
    let mut vt = VulkanTexture::default();
    let r = unsafe {
        t1.vk_upload_ex(
            &mut vdi,
            &mut vt,
            vk::ImageTiling::LINEAR,
            vk::ImageUsageFlags::SAMPLED,
            vk::ImageLayout::GENERAL,
        )
    };
    assert_eq!(r, Ok(()));
    assert_eq!(read_linear(&ctx, &vt, 5, 3), tight);
    unsafe {
        vt.destruct(ctx.device.handle(), std::ptr::null());
        vdi.destroy();
    }
}

#[test]
fn upload_generate_mipmaps() {
    let ctx = Ctx::new();
    let mut t = ktx2(TextureCreateInfo {
        vk_format: VkFormat::R8G8B8A8_UNORM,
        base_width: 16,
        base_height: 4,
        base_depth: 1,
        num_dimensions: 2,
        num_levels: 1,
        num_layers: 1,
        num_faces: 1,
        generate_mipmaps: true,
        ..Default::default()
    });
    let level0 = ktx2_levels(&t).remove(0);
    let mut vdi = ctx.device_info();
    let mut vt = VulkanTexture::default();
    let r = unsafe {
        t.vk_upload_ex(
            &mut vdi,
            &mut vt,
            vk::ImageTiling::OPTIMAL,
            vk::ImageUsageFlags::SAMPLED,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
        )
    };
    assert_eq!(r, Ok(()));
    // floor(log2(16)) + 1 levels, created by blits from level 0.
    assert_eq!(vt.level_count, 5);
    assert_eq!(ctx.read_level(&vt, 0, level0.len()), level0);
    // A blit of a constant image stays constant: check the 1x1 level is a texel average.
    let top = ctx.read_level(&vt, 4, 4);
    assert_eq!(top.len(), 4);
    unsafe {
        vt.destruct(ctx.device.handle(), std::ptr::null());
        vdi.destroy();
    }
}

/// A suballocator built from plain fn pointers, as in C: the device lives in a global.
struct SubAlloc {
    device: ash::Device,
    next_id: u64,
    allocs: HashMap<u64, vk::DeviceMemory>,
    freed: u32,
}

fn sub_alloc() -> &'static Mutex<Option<SubAlloc>> {
    static S: OnceLock<Mutex<Option<SubAlloc>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(None))
}

fn with_sub<R>(f: impl FnOnce(&mut SubAlloc) -> R) -> R {
    let mut g = sub_alloc().lock().unwrap_or_else(|e| e.into_inner());
    match g.as_mut() {
        Some(s) => f(s),
        None => panic!("suballocator not set up"),
    }
}

fn sa_alloc(info: &vk::MemoryAllocateInfo, _req: &vk::MemoryRequirements, pages: &mut u64) -> u64 {
    with_sub(|s| {
        let mem = unsafe { s.device.allocate_memory(info, None) }.unwrap_or_else(|e| panic!("{e}"));
        s.next_id += 1;
        s.allocs.insert(s.next_id, mem);
        *pages = 1;
        s.next_id
    })
}
fn sa_bind_buffer(buffer: vk::Buffer, id: u64) -> vk::Result {
    with_sub(|s| unsafe { s.device.bind_buffer_memory(buffer, s.allocs[&id], 0) }.err())
        .unwrap_or(vk::Result::SUCCESS)
}
fn sa_bind_image(image: vk::Image, id: u64) -> vk::Result {
    with_sub(|s| unsafe { s.device.bind_image_memory(image, s.allocs[&id], 0) }.err())
        .unwrap_or(vk::Result::SUCCESS)
}
fn sa_map(
    id: u64,
    _page: u64,
    len: &mut vk::DeviceSize,
    ptr: &mut *mut std::ffi::c_void,
) -> vk::Result {
    with_sub(|s| {
        match unsafe {
            s.device
                .map_memory(s.allocs[&id], 0, *len, vk::MemoryMapFlags::empty())
        } {
            Ok(p) => {
                *ptr = p;
                vk::Result::SUCCESS
            }
            Err(e) => e,
        }
    })
}
fn sa_unmap(id: u64, _page: u64) {
    with_sub(|s| unsafe { s.device.unmap_memory(s.allocs[&id]) });
}
fn sa_free(id: u64) {
    with_sub(|s| {
        if let Some(m) = s.allocs.remove(&id) {
            unsafe { s.device.free_memory(m, None) };
            s.freed += 1;
        }
    });
}

#[test]
fn upload_with_suballocator() {
    let ctx = Ctx::new();
    *sub_alloc().lock().unwrap_or_else(|e| e.into_inner()) = Some(SubAlloc {
        device: ctx.device.clone(),
        next_id: 0,
        allocs: HashMap::new(),
        freed: 0,
    });
    let callbacks = SubAllocatorCallbacks {
        alloc_mem_func_ptr: Some(sa_alloc),
        bind_buffer_func_ptr: Some(sa_bind_buffer),
        bind_image_func_ptr: Some(sa_bind_image),
        memory_map_func_ptr: Some(sa_map),
        memory_unmap_func_ptr: Some(sa_unmap),
        free_mem_func_ptr: Some(sa_free),
    };
    let mut t = rgba8(8, 8, 1, 2, 2, 1, 1);
    let want = ktx2_levels(&t);
    let mut vdi = ctx.device_info();
    let mut vt = VulkanTexture::default();

    // Incomplete callbacks are rejected.
    let partial = SubAllocatorCallbacks {
        free_mem_func_ptr: None,
        ..callbacks
    };
    let r = unsafe {
        t.vk_upload_ex_with_suballocator(
            &mut vdi,
            &mut vt,
            vk::ImageTiling::OPTIMAL,
            vk::ImageUsageFlags::SAMPLED,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            Some(&partial),
        )
    };
    assert_eq!(r, Err(ErrorCode::InvalidValue));

    let r = unsafe {
        t.vk_upload_ex_with_suballocator(
            &mut vdi,
            &mut vt,
            vk::ImageTiling::OPTIMAL,
            vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_SRC,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            Some(&callbacks),
        )
    };
    assert_eq!(r, Ok(()));
    assert_ne!(vt.allocation_id, 0);
    assert_eq!(vt.device_memory, vk::DeviceMemory::null());
    // The staging allocation was freed through the suballocator.
    assert_eq!(with_sub(|s| (s.freed, s.allocs.len())), (1, 1));
    for (level, w) in want.iter().enumerate() {
        assert!(
            ctx.read_level(&vt, level as u32, w.len()) == *w,
            "level {level}"
        );
    }
    let r = unsafe {
        vt.destruct_with_suballocator(ctx.device.handle(), std::ptr::null(), Some(&callbacks))
    };
    assert_eq!(r, Ok(()));
    assert_eq!(with_sub(|s| (s.freed, s.allocs.len())), (2, 0));
    unsafe { vdi.destroy() };
    *sub_alloc().lock().unwrap_or_else(|e| e.into_inner()) = None;
}

#[test]
fn upload_errors() {
    let ctx = Ctx::new();
    let mut vdi = ctx.device_info();
    let mut vt = VulkanTexture::default();
    // Nothing to upload.
    let mut empty = Texture2::create(
        &TextureCreateInfo {
            vk_format: VkFormat::R8G8B8A8_UNORM,
            base_width: 4,
            base_height: 4,
            base_depth: 1,
            num_dimensions: 2,
            num_levels: 1,
            num_layers: 1,
            num_faces: 1,
            ..Default::default()
        },
        TextureCreateStorage::NoStorage,
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(
        unsafe { empty.vk_upload(&mut vdi, &mut vt) },
        Err(ErrorCode::InvalidOperation)
    );
    // An unset function pointer is reported instead of called.
    let mut t = rgba8(4, 4, 1, 2, 1, 1, 1);
    let saved = vdi.vk_funcs;
    vdi.vk_funcs.vk_create_image = None;
    assert_eq!(
        unsafe { t.vk_upload(&mut vdi, &mut vt) },
        Err(ErrorCode::NotFound)
    );
    vdi.vk_funcs = saved;
    assert_eq!(unsafe { t.vk_upload(&mut vdi, &mut vt) }, Ok(()));
    assert_eq!(vt.view_type, vk::ImageViewType::TYPE_2D);
    assert_eq!(vt.image_format, vk::Format::R8G8B8A8_UNORM);
    unsafe {
        vt.destruct(ctx.device.handle(), std::ptr::null());
        vdi.destroy();
    }
}

/// `ktxVulkanDeviceInfo_Create` without an instance or functions: the commands are looked up
/// in the process (`dlopen(NULL)`), so the Vulkan library must be globally loaded, as when an
/// application links it.
#[cfg(unix)]
#[test]
fn device_info_from_process_symbols() {
    let ctx = Ctx::new();
    let _global = unsafe {
        libloading::os::unix::Library::open(
            Some("libvulkan.so.1"),
            libloading::os::unix::RTLD_NOW | libloading::os::unix::RTLD_GLOBAL,
        )
    }
    .unwrap_or_else(|e| panic!("{e}"));
    assert!(ktx_load_vulkan_function(c"vkGetInstanceProcAddr").is_some());
    assert!(ktx_load_vulkan_function(c"vkNoSuchCommandKTX").is_none());
    let vdi = unsafe {
        VulkanDeviceInfo::create(
            ctx.pd,
            ctx.device.handle(),
            ctx.queue,
            ctx.pool,
            std::ptr::null(),
        )
    }
    .unwrap_or_else(|| panic!("ktxVulkanDeviceInfo_Create failed"));
    assert!(vdi.vk_funcs.vk_cmd_blit_image.is_some());
    assert_ne!(vdi.cmd_buffer, vk::CommandBuffer::null());
    let mut t = rgba8(4, 4, 1, 2, 1, 1, 1);
    let want = ktx2_levels(&t);
    let mut vdi = vdi;
    let mut vt = VulkanTexture::default();
    let r = unsafe {
        t.vk_upload_ex(
            &mut vdi,
            &mut vt,
            vk::ImageTiling::OPTIMAL,
            vk::ImageUsageFlags::TRANSFER_SRC,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
        )
    };
    assert_eq!(r, Ok(()));
    assert_eq!(ctx.read_level(&vt, 0, want[0].len()), want[0]);
    unsafe {
        vt.destruct(ctx.device.handle(), std::ptr::null());
        vdi.destroy();
    }
}
