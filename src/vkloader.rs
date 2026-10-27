// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software and modified; see NOTICE.md.
//! Port of `lib/src/vkloader.c`, `lib/src/vk_funcs.c` and `lib/include/ktxvulkan.h`: creates
//! Vulkan images from KTX textures.
//!
//! This is the only module that may use `unsafe`: every call goes through raw Vulkan function
//! pointers on handles supplied by the application. The public functions taking Vulkan handles
//! are `unsafe fn`; their contract is the one of the Vulkan calls they make (valid handles,
//! externally synchronized queue / command pool, a live `pAllocator`).
//!
//! As in C the Vulkan library is never loaded by libktx itself: `ktx_load_vulkan_library` looks
//! for it in the running process (`dlopen(NULL)` / `GetModuleHandleEx("vulkan-1.dll")`), so the
//! application must have linked or globally loaded it. Applications that pass an instance and
//! `vkGetInstanceProcAddr` in [`VulkanFunctions`] do not need that.
//!
//! `VK_CHECK_RESULT` is a no-op in release builds of the C library; those results are ignored
//! here too. Calls through a function pointer that was never loaded return
//! `KTX_NOT_FOUND` instead of crashing, and writes to mapped memory are bounds checked.
#![allow(unsafe_code)]

use std::ffi::{CStr, c_void};
use std::sync::OnceLock;

pub use ash::vk;

use crate::error::{ErrorCode, KtxResult};
use crate::texture::{ClassId, KtxTexture};
use crate::texture_any::AnyTexture;
use crate::texture1::Texture1;
use crate::texture2::Texture2;
use crate::texture2::lcm4;

const DEFAULT_FENCE_TIMEOUT: u64 = 100_000_000_000;
const KTX_GL_UNPACK_ALIGNMENT: u32 = 4;

/// `_KTX_PADN`.
fn padn(n: u32, nbytes: u64) -> u64 {
    (n as f32 * (nbytes as f32 / n as f32).ceil()) as u64
}

/// `_KTX_PAD_UNPACK_ALIGN_LEN`.
fn pad_unpack_align_len(nbytes: u32) -> u32 {
    (padn(KTX_GL_UNPACK_ALIGNMENT, nbytes as u64) as u32).wrapping_sub(nbytes)
}

/// `_KTX_PAD_UNPACK_ALIGN`.
fn pad_unpack_align(nbytes: u32) -> u32 {
    padn(KTX_GL_UNPACK_ALIGNMENT, nbytes as u64) as u32
}

/// Port of `ktxVulkanFunctions`: the Vulkan commands the loader uses. Unset entries are loaded
/// by [`VulkanDeviceInfo::construct_ex`].
#[derive(Clone, Copy, Default)]
pub struct VulkanFunctions {
    // These are functions pointers we need to perform our vulkan duties.
    pub vk_get_instance_proc_addr: Option<vk::PFN_vkGetInstanceProcAddr>,
    pub vk_get_device_proc_addr: Option<vk::PFN_vkGetDeviceProcAddr>,
    // These we optionally specify
    pub vk_allocate_command_buffers: Option<vk::PFN_vkAllocateCommandBuffers>,
    pub vk_allocate_memory: Option<vk::PFN_vkAllocateMemory>,
    pub vk_begin_command_buffer: Option<vk::PFN_vkBeginCommandBuffer>,
    pub vk_bind_buffer_memory: Option<vk::PFN_vkBindBufferMemory>,
    pub vk_bind_image_memory: Option<vk::PFN_vkBindImageMemory>,
    pub vk_cmd_blit_image: Option<vk::PFN_vkCmdBlitImage>,
    pub vk_cmd_copy_buffer_to_image: Option<vk::PFN_vkCmdCopyBufferToImage>,
    pub vk_cmd_pipeline_barrier: Option<vk::PFN_vkCmdPipelineBarrier>,
    pub vk_create_image: Option<vk::PFN_vkCreateImage>,
    pub vk_destroy_image: Option<vk::PFN_vkDestroyImage>,
    pub vk_create_buffer: Option<vk::PFN_vkCreateBuffer>,
    pub vk_destroy_buffer: Option<vk::PFN_vkDestroyBuffer>,
    pub vk_create_fence: Option<vk::PFN_vkCreateFence>,
    pub vk_destroy_fence: Option<vk::PFN_vkDestroyFence>,
    pub vk_end_command_buffer: Option<vk::PFN_vkEndCommandBuffer>,
    pub vk_free_command_buffers: Option<vk::PFN_vkFreeCommandBuffers>,
    pub vk_free_memory: Option<vk::PFN_vkFreeMemory>,
    pub vk_get_buffer_memory_requirements: Option<vk::PFN_vkGetBufferMemoryRequirements>,
    pub vk_get_image_memory_requirements: Option<vk::PFN_vkGetImageMemoryRequirements>,
    pub vk_get_image_subresource_layout: Option<vk::PFN_vkGetImageSubresourceLayout>,
    pub vk_get_physical_device_image_format_properties:
        Option<vk::PFN_vkGetPhysicalDeviceImageFormatProperties>,
    pub vk_get_physical_device_format_properties:
        Option<vk::PFN_vkGetPhysicalDeviceFormatProperties>,
    pub vk_get_physical_device_memory_properties:
        Option<vk::PFN_vkGetPhysicalDeviceMemoryProperties>,
    pub vk_map_memory: Option<vk::PFN_vkMapMemory>,
    pub vk_queue_submit: Option<vk::PFN_vkQueueSubmit>,
    pub vk_queue_wait_idle: Option<vk::PFN_vkQueueWaitIdle>,
    pub vk_unmap_memory: Option<vk::PFN_vkUnmapMemory>,
    pub vk_wait_for_fences: Option<vk::PFN_vkWaitForFences>,
}

/// Port of `ktxVulkanTexture`: a Vulkan image created by the loader.
#[derive(Clone, Copy, Default, Debug)]
pub struct VulkanTexture {
    /// Pointer to vkDestroyImage function
    pub vk_destroy_image: Option<vk::PFN_vkDestroyImage>,
    /// Pointer to vkFreeMemory function
    pub vk_free_memory: Option<vk::PFN_vkFreeMemory>,
    /// Handle to the Vulkan image created by the loader.
    pub image: vk::Image,
    /// Format of the image data.
    pub image_format: vk::Format,
    /// Layout of the created image. Has the same value as `final_layout` passed to the loader.
    pub image_layout: vk::ImageLayout,
    /// The memory (sub)allocation for the image on the Vulkan device. Will not be used with
    /// suballocators.
    pub device_memory: vk::DeviceMemory,
    /// ViewType corresponding to `image`. Reflects the dimensionality, cubeness and arrayness
    /// of the image.
    pub view_type: vk::ImageViewType,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    /// The number of MIP levels in the image.
    pub level_count: u32,
    /// The number of array layers in the image.
    pub layer_count: u32,
    /// An id referencing suballocation(s).
    pub allocation_id: u64,
}

/// `ktxVulkanTexture_subAllocatorAllocMemFuncPtr`: returns the allocation id (0 on failure) and
/// sets the page count.
pub type SubAllocatorAllocMemFn = fn(
    alloc_info: &vk::MemoryAllocateInfo,
    mem_req: &vk::MemoryRequirements,
    page_count: &mut u64,
) -> u64;
/// `ktxVulkanTexture_subAllocatorBindBufferFuncPtr`.
pub type SubAllocatorBindBufferFn = fn(buffer: vk::Buffer, alloc_id: u64) -> vk::Result;
/// `ktxVulkanTexture_subAllocatorBindImageFuncPtr`.
pub type SubAllocatorBindImageFn = fn(image: vk::Image, alloc_id: u64) -> vk::Result;
/// `ktxVulkanTexture_subAllocatorMemoryMapFuncPtr`: maps a page, setting its length and address.
pub type SubAllocatorMemoryMapFn = fn(
    alloc_id: u64,
    page_number: u64,
    map_length: &mut vk::DeviceSize,
    data_ptr: &mut *mut c_void,
) -> vk::Result;
/// `ktxVulkanTexture_subAllocatorMemoryUnmapFuncPtr`.
pub type SubAllocatorMemoryUnmapFn = fn(alloc_id: u64, page_number: u64);
/// `ktxVulkanTexture_subAllocatorFreeMemFuncPtr`.
pub type SubAllocatorFreeMemFn = fn(alloc_id: u64);

/// Port of `ktxVulkanTexture_subAllocatorCallbacks`.
#[derive(Clone, Copy, Default)]
pub struct SubAllocatorCallbacks {
    pub alloc_mem_func_ptr: Option<SubAllocatorAllocMemFn>,
    pub bind_buffer_func_ptr: Option<SubAllocatorBindBufferFn>,
    pub bind_image_func_ptr: Option<SubAllocatorBindImageFn>,
    pub memory_map_func_ptr: Option<SubAllocatorMemoryMapFn>,
    pub memory_unmap_func_ptr: Option<SubAllocatorMemoryUnmapFn>,
    pub free_mem_func_ptr: Option<SubAllocatorFreeMemFn>,
}

/// The callbacks of a complete [`SubAllocatorCallbacks`].
#[derive(Clone, Copy)]
struct SubAllocator {
    alloc_mem: SubAllocatorAllocMemFn,
    bind_buffer: SubAllocatorBindBufferFn,
    bind_image: SubAllocatorBindImageFn,
    memory_map: SubAllocatorMemoryMapFn,
    memory_unmap: SubAllocatorMemoryUnmapFn,
    free_mem: SubAllocatorFreeMemFn,
}

impl SubAllocatorCallbacks {
    /// All callbacks set, or `KTX_INVALID_VALUE` if only some are.
    fn complete(&self) -> KtxResult<SubAllocator> {
        match (
            self.alloc_mem_func_ptr,
            self.bind_buffer_func_ptr,
            self.bind_image_func_ptr,
            self.memory_map_func_ptr,
            self.memory_unmap_func_ptr,
            self.free_mem_func_ptr,
        ) {
            (Some(a), Some(bb), Some(bi), Some(m), Some(u), Some(f)) => Ok(SubAllocator {
                alloc_mem: a,
                bind_buffer: bb,
                bind_image: bi,
                memory_map: m,
                memory_unmap: u,
                free_mem: f,
            }),
            _ => Err(ErrorCode::InvalidValue),
        }
    }
}

/// Port of `ktxVulkanDeviceInfo`: the Vulkan device on which to create images.
pub struct VulkanDeviceInfo {
    /// Instance used to communicate with vulkan.
    pub instance: vk::Instance,
    /// Handle of the physical device.
    pub physical_device: vk::PhysicalDevice,
    /// Handle of the logical device.
    pub device: vk::Device,
    /// Handle to the queue to which to submit commands.
    pub queue: vk::Queue,
    /// Handle of the cmdBuffer to use.
    pub cmd_buffer: vk::CommandBuffer,
    /// Handle of the command pool from which to allocate the command buffer.
    pub cmd_pool: vk::CommandPool,
    /// Pointer to the allocator to use for the command buffer and created images (may be null).
    pub p_allocator: *const vk::AllocationCallbacks<'static>,
    /// Memory properties of the Vulkan physical device.
    pub device_memory_properties: vk::PhysicalDeviceMemoryProperties,
    /// The functions needed to operate functions
    pub vk_funcs: VulkanFunctions,
}

impl Default for VulkanDeviceInfo {
    fn default() -> Self {
        VulkanDeviceInfo {
            instance: vk::Instance::null(),
            physical_device: vk::PhysicalDevice::null(),
            device: vk::Device::null(),
            queue: vk::Queue::null(),
            cmd_buffer: vk::CommandBuffer::null(),
            cmd_pool: vk::CommandPool::null(),
            p_allocator: std::ptr::null(),
            device_memory_properties: vk::PhysicalDeviceMemoryProperties::default(),
            vk_funcs: VulkanFunctions::default(),
        }
    }
}

/// `ktxVulkanModuleHandle`: the process (or `vulkan-1.dll`) searched for Vulkan commands.
static VULKAN_MODULE: OnceLock<Option<libloading::Library>> = OnceLock::new();

#[cfg(unix)]
fn get_vulkan_module_handle() -> Option<libloading::Library> {
    // Using NULL returns a handle that can be used to search the process that loaded us and any
    // other libraries it has loaded.
    Some(libloading::os::unix::Library::this().into())
}

#[cfg(windows)]
fn get_vulkan_module_handle() -> Option<libloading::Library> {
    libloading::os::windows::Library::open_already_loaded("vulkan-1.dll")
        .ok()
        .map(Into::into)
}

/// Port of `ktxLoadVulkanLibrary`.
pub fn ktx_load_vulkan_library() -> KtxResult<()> {
    match VULKAN_MODULE.get_or_init(get_vulkan_module_handle) {
        Some(_) => Ok(()),
        None => {
            eprintln!("Vulkan lib not linked or loaded by application.");
            Err(ErrorCode::LibraryNotLinked)
        }
    }
}

/// Port of `ktxLoadVulkanFunction`: looks a Vulkan command up in the process.
pub fn ktx_load_vulkan_function(name: &CStr) -> vk::PFN_vkVoidFunction {
    ktx_load_vulkan_library().ok()?;
    let module = VULKAN_MODULE.get()?.as_ref()?;
    // SAFETY: the symbol is a Vulkan command, called only through its PFN type.
    let pfn = unsafe { module.get::<unsafe extern "system" fn()>(name.to_bytes_with_nul()) }
        .ok()
        .map(|s| *s);
    if pfn.is_none() {
        eprintln!("Couldn't load Vulkan command: {}", name.to_string_lossy());
    }
    pfn
}

/// Reinterprets a `PFN_vkVoidFunction` as the command's real type.
macro_rules! cast_pfn {
    ($pfn:expr, $t:ty) => {
        // SAFETY: the pointer was returned for the command named like `$t`.
        $pfn.map(|f| unsafe { std::mem::transmute::<unsafe extern "system" fn(), $t>(f) })
    };
}

/// Calls a loaded command or returns `KTX_NOT_FOUND`.
macro_rules! vkf {
    ($funcs:expr, $name:ident) => {
        match $funcs.$name {
            Some(f) => f,
            None => return Err(ErrorCode::NotFound),
        }
    };
}

impl VulkanDeviceInfo {
    /// Port of `ktxVulkanDeviceInfo_Create`.
    ///
    /// # Safety
    /// See [`VulkanDeviceInfo::construct_ex`].
    pub unsafe fn create(
        physical_device: vk::PhysicalDevice,
        device: vk::Device,
        queue: vk::Queue,
        cmd_pool: vk::CommandPool,
        p_allocator: *const vk::AllocationCallbacks<'static>,
    ) -> Option<Box<VulkanDeviceInfo>> {
        // SAFETY: forwarded contract.
        unsafe {
            Self::create_ex(
                vk::Instance::null(),
                physical_device,
                device,
                queue,
                cmd_pool,
                p_allocator,
                None,
            )
        }
    }

    /// Port of `ktxVulkanDeviceInfo_CreateEx`. Returns `None` if construction fails.
    ///
    /// # Safety
    /// See [`VulkanDeviceInfo::construct_ex`].
    pub unsafe fn create_ex(
        instance: vk::Instance,
        physical_device: vk::PhysicalDevice,
        device: vk::Device,
        queue: vk::Queue,
        cmd_pool: vk::CommandPool,
        p_allocator: *const vk::AllocationCallbacks<'static>,
        p_funcs: Option<&VulkanFunctions>,
    ) -> Option<Box<VulkanDeviceInfo>> {
        let mut newvdi = Box::new(VulkanDeviceInfo::default());
        // SAFETY: forwarded contract.
        unsafe {
            newvdi
                .construct_ex(
                    instance,
                    physical_device,
                    device,
                    queue,
                    cmd_pool,
                    p_allocator,
                    p_funcs,
                )
                .ok()?;
        }
        Some(newvdi)
    }

    /// Port of `ktxVulkanDeviceInfo_Construct`.
    ///
    /// # Safety
    /// See [`VulkanDeviceInfo::construct_ex`].
    pub unsafe fn construct(
        &mut self,
        physical_device: vk::PhysicalDevice,
        device: vk::Device,
        queue: vk::Queue,
        cmd_pool: vk::CommandPool,
        p_allocator: *const vk::AllocationCallbacks<'static>,
    ) -> KtxResult<()> {
        // SAFETY: forwarded contract.
        unsafe {
            self.construct_ex(
                vk::Instance::null(),
                physical_device,
                device,
                queue,
                cmd_pool,
                p_allocator,
                None,
            )
        }
    }

    /// Port of `ktxVulkanDeviceInfo_ConstructEx`: loads the missing functions and allocates the
    /// command buffer used for uploads.
    ///
    /// # Safety
    /// The handles must be valid and belong together, `p_allocator` must be null or outlive
    /// `self`, and the functions in `p_functions` must be Vulkan commands of `instance` /
    /// `device`.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn construct_ex(
        &mut self,
        instance: vk::Instance,
        physical_device: vk::PhysicalDevice,
        device: vk::Device,
        queue: vk::Queue,
        cmd_pool: vk::CommandPool,
        p_allocator: *const vk::AllocationCallbacks<'static>,
        p_functions: Option<&VulkanFunctions>,
    ) -> KtxResult<()> {
        let mut funcs = p_functions.copied().unwrap_or_default();

        macro_rules! load_ext_func {
            ($member:ident, $fun:literal, $t:ty) => {
                if funcs.$member.is_none() {
                    funcs.$member = cast_pfn!(ktx_load_vulkan_function($fun), $t);
                    if funcs.$member.is_none() {
                        return Err(ErrorCode::NotFound);
                    }
                }
            };
        }
        macro_rules! load_inst_func {
            ($member:ident, $fun:literal, $t:ty) => {
                if funcs.$member.is_none() {
                    let gipa = vkf!(funcs, vk_get_instance_proc_addr);
                    // SAFETY: valid instance (or null for global commands) and command name.
                    funcs.$member = cast_pfn!(unsafe { gipa(instance, $fun.as_ptr()) }, $t);
                    if funcs.$member.is_none() {
                        return Err(ErrorCode::NotFound);
                    }
                }
            };
        }
        macro_rules! load_device_func {
            ($member:ident, $fun:literal, $t:ty) => {
                if funcs.$member.is_none() {
                    let gdpa = vkf!(funcs, vk_get_device_proc_addr);
                    // SAFETY: valid device and command name.
                    funcs.$member = cast_pfn!(unsafe { gdpa(device, $fun.as_ptr()) }, $t);
                    if funcs.$member.is_none() {
                        return Err(ErrorCode::NotFound);
                    }
                }
            };
        }

        self.instance = instance;
        self.physical_device = physical_device;
        self.device = device;
        self.queue = queue;
        self.cmd_pool = cmd_pool;
        self.p_allocator = p_allocator;

        if instance == vk::Instance::null() || p_functions.is_none() {
            // This is the old behavior, where no functions where specified. We take a shot at
            // loading them dynamically.
            ktx_load_vulkan_library()?;
            // If pFunctions are null, then we try to load the minimum number of required
            // functions.
            if p_functions.is_none() {
                load_ext_func!(
                    vk_get_instance_proc_addr,
                    c"vkGetInstanceProcAddr",
                    vk::PFN_vkGetInstanceProcAddr
                );
            }
            // If we have no instance, we will need to bind the instance-level functions, as we
            // won't be able to load them through vkGetInstanceProcAddr, and we need to load
            // vkGetDeviceProcAddr.
            if instance == vk::Instance::null() {
                load_ext_func!(
                    vk_get_device_proc_addr,
                    c"vkGetDeviceProcAddr",
                    vk::PFN_vkGetDeviceProcAddr
                );
                // These require an instance, which we don't have.
                load_ext_func!(
                    vk_get_physical_device_format_properties,
                    c"vkGetPhysicalDeviceFormatProperties",
                    vk::PFN_vkGetPhysicalDeviceFormatProperties
                );
                load_ext_func!(
                    vk_get_physical_device_image_format_properties,
                    c"vkGetPhysicalDeviceImageFormatProperties",
                    vk::PFN_vkGetPhysicalDeviceImageFormatProperties
                );
                load_ext_func!(
                    vk_get_physical_device_memory_properties,
                    c"vkGetPhysicalDeviceMemoryProperties",
                    vk::PFN_vkGetPhysicalDeviceMemoryProperties
                );
            }
        }

        // We now load the rest of the functions in.
        load_inst_func!(
            vk_get_physical_device_format_properties,
            c"vkGetPhysicalDeviceFormatProperties",
            vk::PFN_vkGetPhysicalDeviceFormatProperties
        );
        load_inst_func!(
            vk_get_physical_device_memory_properties,
            c"vkGetPhysicalDeviceMemoryProperties",
            vk::PFN_vkGetPhysicalDeviceMemoryProperties
        );
        load_inst_func!(
            vk_get_physical_device_image_format_properties,
            c"vkGetPhysicalDeviceImageFormatProperties",
            vk::PFN_vkGetPhysicalDeviceImageFormatProperties
        );

        // We now get to device functions.
        load_inst_func!(
            vk_get_device_proc_addr,
            c"vkGetDeviceProcAddr",
            vk::PFN_vkGetDeviceProcAddr
        );
        load_device_func!(
            vk_allocate_command_buffers,
            c"vkAllocateCommandBuffers",
            vk::PFN_vkAllocateCommandBuffers
        );
        load_device_func!(
            vk_free_command_buffers,
            c"vkFreeCommandBuffers",
            vk::PFN_vkFreeCommandBuffers
        );
        load_device_func!(
            vk_allocate_memory,
            c"vkAllocateMemory",
            vk::PFN_vkAllocateMemory
        );
        load_device_func!(vk_free_memory, c"vkFreeMemory", vk::PFN_vkFreeMemory);
        load_device_func!(
            vk_begin_command_buffer,
            c"vkBeginCommandBuffer",
            vk::PFN_vkBeginCommandBuffer
        );
        load_device_func!(
            vk_end_command_buffer,
            c"vkEndCommandBuffer",
            vk::PFN_vkEndCommandBuffer
        );
        load_device_func!(
            vk_bind_buffer_memory,
            c"vkBindBufferMemory",
            vk::PFN_vkBindBufferMemory
        );
        load_device_func!(
            vk_bind_image_memory,
            c"vkBindImageMemory",
            vk::PFN_vkBindImageMemory
        );
        load_device_func!(vk_cmd_blit_image, c"vkCmdBlitImage", vk::PFN_vkCmdBlitImage);
        load_device_func!(
            vk_cmd_copy_buffer_to_image,
            c"vkCmdCopyBufferToImage",
            vk::PFN_vkCmdCopyBufferToImage
        );
        load_device_func!(
            vk_cmd_pipeline_barrier,
            c"vkCmdPipelineBarrier",
            vk::PFN_vkCmdPipelineBarrier
        );
        load_device_func!(vk_create_image, c"vkCreateImage", vk::PFN_vkCreateImage);
        load_device_func!(vk_destroy_image, c"vkDestroyImage", vk::PFN_vkDestroyImage);
        load_device_func!(vk_create_buffer, c"vkCreateBuffer", vk::PFN_vkCreateBuffer);
        load_device_func!(
            vk_destroy_buffer,
            c"vkDestroyBuffer",
            vk::PFN_vkDestroyBuffer
        );
        load_device_func!(vk_create_fence, c"vkCreateFence", vk::PFN_vkCreateFence);
        load_device_func!(vk_destroy_fence, c"vkDestroyFence", vk::PFN_vkDestroyFence);
        load_device_func!(
            vk_wait_for_fences,
            c"vkWaitForFences",
            vk::PFN_vkWaitForFences
        );
        load_device_func!(vk_map_memory, c"vkMapMemory", vk::PFN_vkMapMemory);
        load_device_func!(vk_unmap_memory, c"vkUnmapMemory", vk::PFN_vkUnmapMemory);
        load_device_func!(vk_queue_submit, c"vkQueueSubmit", vk::PFN_vkQueueSubmit);
        load_device_func!(
            vk_queue_wait_idle,
            c"vkQueueWaitIdle",
            vk::PFN_vkQueueWaitIdle
        );
        load_device_func!(
            vk_get_buffer_memory_requirements,
            c"vkGetBufferMemoryRequirements",
            vk::PFN_vkGetBufferMemoryRequirements
        );
        load_device_func!(
            vk_get_image_memory_requirements,
            c"vkGetImageMemoryRequirements",
            vk::PFN_vkGetImageMemoryRequirements
        );
        load_device_func!(
            vk_get_image_subresource_layout,
            c"vkGetImageSubresourceLayout",
            vk::PFN_vkGetImageSubresourceLayout
        );

        self.vk_funcs = funcs;

        let get_mem_props = vkf!(self.vk_funcs, vk_get_physical_device_memory_properties);
        let allocate_command_buffers = vkf!(self.vk_funcs, vk_allocate_command_buffers);
        // SAFETY: valid physical device; the output is a plain struct.
        unsafe { get_mem_props(physical_device, &mut self.device_memory_properties) };

        // Use a separate command buffer for texture loading. Needed for submitting image
        // barriers and converting tilings.
        let cmd_buf_info = vk::CommandBufferAllocateInfo {
            command_pool: cmd_pool,
            level: vk::CommandBufferLevel::PRIMARY,
            command_buffer_count: 1,
            ..Default::default()
        };
        // SAFETY: valid device and pool; one command buffer is written.
        let result =
            unsafe { allocate_command_buffers(device, &cmd_buf_info, &mut self.cmd_buffer) };
        if result != vk::Result::SUCCESS {
            return Err(ErrorCode::OutOfMemory); // TODO: Consider an equivalent to pGlError
        }
        Ok(())
    }

    /// Port of `ktxVulkanDeviceInfo_Destruct`: frees the command buffer.
    ///
    /// # Safety
    /// The device and command pool must still be valid and the command buffer not in use.
    pub unsafe fn destruct(&mut self) {
        if let Some(free_command_buffers) = self.vk_funcs.vk_free_command_buffers {
            // SAFETY: forwarded contract.
            unsafe { free_command_buffers(self.device, self.cmd_pool, 1, &self.cmd_buffer) };
        }
    }

    /// Port of `ktxVulkanDeviceInfo_Destroy`.
    ///
    /// # Safety
    /// See [`VulkanDeviceInfo::destruct`].
    pub unsafe fn destroy(mut self: Box<Self>) {
        // SAFETY: forwarded contract.
        unsafe { self.destruct() };
    }

    /// `ktxVulkanDeviceInfo_getMemoryType`: memory type index for an allocation.
    fn get_memory_type(&self, mut type_bits: u32, properties: vk::MemoryPropertyFlags) -> u32 {
        for (i, t) in self
            .device_memory_properties
            .memory_types
            .iter()
            .enumerate()
        {
            if (type_bits & 1) == 1 && t.property_flags.contains(properties) {
                return i as u32;
            }
            type_bits >>= 1;
        }
        // XXX : throw error
        0
    }
}

/// Host mapped memory; writes are bounds checked.
struct Mapped {
    ptr: *mut u8,
    len: usize,
}

impl Mapped {
    fn slice(&mut self) -> &mut [u8] {
        if self.ptr.is_null() {
            return &mut [];
        }
        // SAFETY: `ptr` is a live mapping of `len` bytes for as long as `self` is used.
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }

    fn write(&mut self, offset: u64, data: &[u8]) -> KtxResult<()> {
        let start = usize::try_from(offset).map_err(|_| ErrorCode::OutOfMemory)?;
        self.slice()
            .get_mut(start..start + data.len())
            .ok_or(ErrorCode::OutOfMemory)?
            .copy_from_slice(data);
        Ok(())
    }
}

/// `user_cbdata_optimal` + `optimalTilingCallback` / `optimalTilingPadCallback`.
struct OptimalCbData<'a> {
    regions: Vec<vk::BufferImageCopy>,
    offset: vk::DeviceSize,
    num_faces: u32,
    num_layers: u32,
    // The following are used only by optimalTilingPadCallback
    dest: &'a mut Mapped,
    element_size: u32,
    num_dimensions: u32,
}

impl OptimalCbData<'_> {
    fn push_region(
        &mut self,
        buffer_offset: u64,
        miplevel: u32,
        face: u32,
        w: u32,
        h: u32,
        d: u32,
    ) {
        self.regions.push(vk::BufferImageCopy {
            buffer_offset,
            // These 2 are expressed in texels.
            buffer_row_length: 0,
            buffer_image_height: 0,
            image_subresource: vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: miplevel,
                base_array_layer: face,
                layer_count: self.num_layers * self.num_faces,
            },
            image_offset: vk::Offset3D { x: 0, y: 0, z: 0 },
            image_extent: vk::Extent3D {
                width: w,
                height: h,
                depth: d,
            },
        });
    }

    /// `optimalTilingCallback`.
    fn optimal_tiling(&mut self, miplevel: u32, face: u32, w: u32, h: u32, d: u32, size: u64) {
        let offset = self.offset;
        self.offset += size;
        self.push_region(offset, miplevel, face, w, h, d);
    }

    /// `optimalTilingPadCallback`.
    #[allow(clippy::too_many_arguments)]
    fn optimal_tiling_pad(
        &mut self,
        miplevel: u32,
        face: u32,
        width: u32,
        height: u32,
        depth: u32,
        face_lod_size: u64,
        pixels: &[u8],
    ) -> KtxResult<()> {
        let buffer_offset = self.offset;
        let mut row_pitch = width * self.element_size;
        // Copy data into staging buffer
        if pad_unpack_align_len(row_pitch) == 0 {
            // No padding. Can copy in bulk.
            let n = usize::try_from(face_lod_size).map_err(|_| ErrorCode::OutOfMemory)?;
            self.dest.write(
                self.offset,
                pixels.get(..n).ok_or(ErrorCode::FileDataError)?,
            )?;
            self.offset += face_lod_size;
        } else {
            // Must remove padding. Copy a row at a time.
            let image_iterations = if self.num_dimensions == 3 {
                depth
            } else if self.num_layers > 1 {
                self.num_layers * self.num_faces
            } else {
                1
            };
            row_pitch = width * self.element_size;
            let padded_row_pitch = pad_unpack_align(row_pitch) as usize;
            let mut src = 0usize;
            for _image in 0..image_iterations {
                for _row in 0..height {
                    let row = pixels
                        .get(src..src + row_pitch as usize)
                        .ok_or(ErrorCode::FileDataError)?;
                    self.dest.write(self.offset, row)?;
                    self.offset += u64::from(row_pitch);
                    src += padded_row_pitch;
                }
            }
        }
        // Round to needed multiples for next region, if necessary.
        if !self.offset.is_multiple_of(u64::from(self.element_size))
            || !self.offset.is_multiple_of(4)
        {
            // Only elementSizes of 1,2 and 3 will bring us here.
            let lcm = lcm4(self.element_size);
            self.offset = padn(lcm, self.offset);
        }
        self.push_region(buffer_offset, miplevel, face, width, height, depth);
        Ok(())
    }
}

/// Per level `ktxTexture_GetRowPitch` / `ktxTexture_GetImageSize`, precomputed so the linear
/// tiling callbacks do not borrow the texture while it is iterated.
struct LevelInfo {
    row_pitch: u32,
    image_size: u64,
}

/// `user_cbdata_linear` + `linearTilingCallback` / `linearTilingPadCallback`.
struct LinearCbData<'a> {
    get_image_subresource_layout: vk::PFN_vkGetImageSubresourceLayout,
    dest_image: vk::Image,
    device: vk::Device,
    dest: &'a mut Mapped,
    levels: Vec<LevelInfo>,
    num_layers: u32,
    num_faces: u32,
    num_dimensions: u32,
    pad: bool,
}

impl LinearCbData<'_> {
    fn subresource_layout(&self, miplevel: u32, face: u32) -> vk::SubresourceLayout {
        let sub_res = vk::ImageSubresource {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            mip_level: miplevel,
            array_layer: face,
        };
        let mut layout = vk::SubresourceLayout::default();
        // Get sub resources layout. Includes row pitch, size, offsets, etc.
        // SAFETY: valid device and linear image; plain output struct.
        unsafe {
            (self.get_image_subresource_layout)(self.device, self.dest_image, &sub_res, &mut layout)
        };
        layout
    }

    #[allow(clippy::too_many_arguments)]
    fn callback(
        &mut self,
        miplevel: u32,
        face: u32,
        height: u32,
        depth: u32,
        face_lod_size: u64,
        pixels: &[u8],
    ) -> KtxResult<()> {
        let sub_res_layout = self.subresource_layout(miplevel, face);
        if !self.pad {
            // linearTilingCallback: copies all images of the miplevel (for array & 3d) or a
            // single face.
            let n = usize::try_from(face_lod_size).map_err(|_| ErrorCode::OutOfMemory)?;
            return self.dest.write(
                sub_res_layout.offset,
                pixels.get(..n).ok_or(ErrorCode::FileDataError)?,
            );
        }

        // linearTilingPadCallback
        let level = self
            .levels
            .get(miplevel as usize)
            .ok_or(ErrorCode::InvalidValue)?;
        let src_row_pitch = level.row_pitch;
        let row_iterations = if sub_res_layout.row_pitch != u64::from(src_row_pitch) {
            height
        } else {
            1
        };
        let mut image_iterations = 1;
        let mut image_size = 0u64;
        let mut image_pitch = 0u64;
        // Arrays, including cube map arrays, or 3D textures
        // Note from the Vulkan spec:
        //  *  arrayPitch is undefined for images that were not created as arrays.
        //  *  depthPitch is defined only for 3D images.
        if self.num_layers > 1 || self.num_dimensions == 3 {
            image_size = level.image_size;
            if self.num_layers > 1 {
                image_pitch = sub_res_layout.array_pitch;
                if image_pitch != image_size {
                    image_iterations = self.num_layers * self.num_faces;
                }
            } else {
                image_pitch = sub_res_layout.depth_pitch;
                if image_pitch != image_size {
                    image_iterations = depth;
                }
            }
        }
        let copy_size = if row_iterations > 1 {
            // Copy the minimum of srcRowPitch, the GL_UNPACK_ALIGNMENT padded size, and
            // subResLayout.rowPitch.
            sub_res_layout.row_pitch.min(u64::from(src_row_pitch))
        } else if image_iterations > 1 {
            face_lod_size / u64::from(image_iterations)
        } else {
            face_lod_size
        };
        let copy_size = usize::try_from(copy_size).map_err(|_| ErrorCode::OutOfMemory)?;

        let mut offset = sub_res_layout.offset;
        // Copy image data to destImage via its mapped memory.
        for image in 0..u64::from(image_iterations) {
            let mut src =
                usize::try_from(image_size * image).map_err(|_| ErrorCode::OutOfMemory)?;
            for _row in 0..row_iterations {
                let data = pixels
                    .get(src..src + copy_size)
                    .ok_or(ErrorCode::FileDataError)?;
                self.dest.write(offset, data)?;
                offset += sub_res_layout.row_pitch;
                src += src_row_pitch as usize;
            }
            offset += image_pitch;
        }
        Ok(())
    }
}

/// The loaded commands used by an upload.
struct UploadFns {
    begin_command_buffer: vk::PFN_vkBeginCommandBuffer,
    end_command_buffer: vk::PFN_vkEndCommandBuffer,
    allocate_memory: vk::PFN_vkAllocateMemory,
    free_memory: vk::PFN_vkFreeMemory,
    bind_buffer_memory: vk::PFN_vkBindBufferMemory,
    bind_image_memory: vk::PFN_vkBindImageMemory,
    map_memory: vk::PFN_vkMapMemory,
    unmap_memory: vk::PFN_vkUnmapMemory,
    create_buffer: vk::PFN_vkCreateBuffer,
    destroy_buffer: vk::PFN_vkDestroyBuffer,
    create_image: vk::PFN_vkCreateImage,
    create_fence: vk::PFN_vkCreateFence,
    destroy_fence: vk::PFN_vkDestroyFence,
    wait_for_fences: vk::PFN_vkWaitForFences,
    queue_submit: vk::PFN_vkQueueSubmit,
    queue_wait_idle: vk::PFN_vkQueueWaitIdle,
    get_buffer_memory_requirements: vk::PFN_vkGetBufferMemoryRequirements,
    get_image_memory_requirements: vk::PFN_vkGetImageMemoryRequirements,
    get_image_subresource_layout: vk::PFN_vkGetImageSubresourceLayout,
    cmd_copy_buffer_to_image: vk::PFN_vkCmdCopyBufferToImage,
    cmd_pipeline_barrier: vk::PFN_vkCmdPipelineBarrier,
    cmd_blit_image: vk::PFN_vkCmdBlitImage,
    get_physical_device_image_format_properties: vk::PFN_vkGetPhysicalDeviceImageFormatProperties,
    get_physical_device_format_properties: vk::PFN_vkGetPhysicalDeviceFormatProperties,
}

impl UploadFns {
    fn new(f: &VulkanFunctions) -> KtxResult<UploadFns> {
        Ok(UploadFns {
            begin_command_buffer: vkf!(f, vk_begin_command_buffer),
            end_command_buffer: vkf!(f, vk_end_command_buffer),
            allocate_memory: vkf!(f, vk_allocate_memory),
            free_memory: vkf!(f, vk_free_memory),
            bind_buffer_memory: vkf!(f, vk_bind_buffer_memory),
            bind_image_memory: vkf!(f, vk_bind_image_memory),
            map_memory: vkf!(f, vk_map_memory),
            unmap_memory: vkf!(f, vk_unmap_memory),
            create_buffer: vkf!(f, vk_create_buffer),
            destroy_buffer: vkf!(f, vk_destroy_buffer),
            create_image: vkf!(f, vk_create_image),
            create_fence: vkf!(f, vk_create_fence),
            destroy_fence: vkf!(f, vk_destroy_fence),
            wait_for_fences: vkf!(f, vk_wait_for_fences),
            queue_submit: vkf!(f, vk_queue_submit),
            queue_wait_idle: vkf!(f, vk_queue_wait_idle),
            get_buffer_memory_requirements: vkf!(f, vk_get_buffer_memory_requirements),
            get_image_memory_requirements: vkf!(f, vk_get_image_memory_requirements),
            get_image_subresource_layout: vkf!(f, vk_get_image_subresource_layout),
            cmd_copy_buffer_to_image: vkf!(f, vk_cmd_copy_buffer_to_image),
            cmd_pipeline_barrier: vkf!(f, vk_cmd_pipeline_barrier),
            cmd_blit_image: vkf!(f, vk_cmd_blit_image),
            get_physical_device_image_format_properties: vkf!(
                f,
                vk_get_physical_device_image_format_properties
            ),
            get_physical_device_format_properties: vkf!(
                f,
                vk_get_physical_device_format_properties
            ),
        })
    }
}

/// Port of `setImageLayout`: records an image layout transition barrier.
///
/// # Safety
/// `cmd_buffer` must be recording and `image` valid.
unsafe fn set_image_layout(
    cmd_pipeline_barrier: vk::PFN_vkCmdPipelineBarrier,
    cmd_buffer: vk::CommandBuffer,
    image: vk::Image,
    old_layout: vk::ImageLayout,
    new_layout: vk::ImageLayout,
    subresource_range: vk::ImageSubresourceRange,
) {
    use vk::AccessFlags as A;
    use vk::ImageLayout as L;
    // Create an image barrier object
    let mut image_memory_barrier = vk::ImageMemoryBarrier {
        // Some default values
        src_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
        dst_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
        old_layout,
        new_layout,
        image,
        subresource_range,
        ..Default::default()
    };

    // Source layouts (old)
    // The source access mask controls actions to be finished on the old layout before it will
    // be transitioned to the new layout.
    image_memory_barrier.src_access_mask = match old_layout {
        // Image layout is undefined (or does not matter). Only valid as initial layout. No
        // flags required.
        L::UNDEFINED => A::empty(),
        // Image is preinitialized. Only valid as initial layout for linear images; preserves
        // memory contents. Make sure host writes have finished.
        L::PREINITIALIZED => A::HOST_WRITE,
        // Image is a color attachment. Make sure writes to the color buffer have finished
        L::COLOR_ATTACHMENT_OPTIMAL => A::COLOR_ATTACHMENT_WRITE,
        // Image is a depth/stencil attachment. Make sure any writes to the depth/stencil buffer
        // have finished.
        L::DEPTH_STENCIL_ATTACHMENT_OPTIMAL => A::DEPTH_STENCIL_ATTACHMENT_WRITE,
        // Image is a transfer source. Make sure any reads from the image have finished
        L::TRANSFER_SRC_OPTIMAL => A::TRANSFER_READ,
        // Image is a transfer destination. Make sure any writes to the image have finished.
        L::TRANSFER_DST_OPTIMAL => A::TRANSFER_WRITE,
        // Image is read by a shader. Make sure any shader reads from the image have finished
        L::SHADER_READ_ONLY_OPTIMAL => A::SHADER_READ,
        // Image may be used for arbitrary read/write access. Make sure any reads and writes to
        // the image have finished.
        L::GENERAL => A::MEMORY_READ | A::MEMORY_WRITE,
        // Value not used by callers, so not supported (asserts in C).
        _ => A::empty(),
    };

    let mut dest_stage_flags = vk::PipelineStageFlags::ALL_COMMANDS;

    // Target layouts (new)
    // The destination access mask controls the dependency for the new image layout.
    match new_layout {
        L::TRANSFER_DST_OPTIMAL => {
            // Image will be used as a transfer destination. Make sure any writes to the image
            // have finished.
            image_memory_barrier.dst_access_mask = A::TRANSFER_WRITE;
        }
        L::TRANSFER_SRC_OPTIMAL => {
            // Image will be used as a transfer source. Make sure any reads from and writes to
            // the image have finished.
            image_memory_barrier.src_access_mask |= A::TRANSFER_READ;
            image_memory_barrier.dst_access_mask = A::TRANSFER_READ;
        }
        L::COLOR_ATTACHMENT_OPTIMAL => {
            // Image will be used as a color attachment. Make sure any writes to the color
            // buffer have finished.
            image_memory_barrier.src_access_mask = A::TRANSFER_READ;
            image_memory_barrier.dst_access_mask = A::COLOR_ATTACHMENT_WRITE;
        }
        L::DEPTH_STENCIL_ATTACHMENT_OPTIMAL => {
            // Image layout will be used as a depth/stencil attachment. Make sure any writes to
            // depth/stencil buffer have finished.
            image_memory_barrier.dst_access_mask = A::DEPTH_STENCIL_ATTACHMENT_WRITE;
        }
        L::SHADER_READ_ONLY_OPTIMAL => {
            // Image will be read in a shader (sampler, input attachment). Make sure any writes
            // to the image have finished.
            if image_memory_barrier.src_access_mask.is_empty() {
                image_memory_barrier.src_access_mask = A::HOST_WRITE | A::TRANSFER_WRITE;
            }
            image_memory_barrier.dst_access_mask = A::SHADER_READ;
            dest_stage_flags =
                vk::PipelineStageFlags::ALL_GRAPHICS | vk::PipelineStageFlags::COMPUTE_SHADER;
        }
        L::GENERAL => {
            // Image may be used for arbitrary read/write access.
            image_memory_barrier.dst_access_mask = A::MEMORY_READ | A::MEMORY_WRITE;
        }
        // Value not used by callers, so not supported (asserts in C).
        _ => {}
    }

    // Put barrier on top of pipeline.
    let src_stage_flags = vk::PipelineStageFlags::ALL_COMMANDS;

    // Add the barrier to the passed command buffer
    // SAFETY: forwarded contract; one barrier is read.
    unsafe {
        cmd_pipeline_barrier(
            cmd_buffer,
            src_stage_flags,
            dest_stage_flags,
            vk::DependencyFlags::empty(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            1,
            &image_memory_barrier,
        )
    };
}

/// Port of `generateMipmaps`: blits each level from the previous one.
///
/// # Safety
/// `vdi.cmd_buffer` must be recording and `vk_texture.image` valid.
unsafe fn generate_mipmaps(
    fns: &UploadFns,
    vk_texture: &VulkanTexture,
    vdi: &VulkanDeviceInfo,
    blit_filter: vk::Filter,
    initial_layout: vk::ImageLayout,
) {
    let mut subresource_range = vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: vk_texture.layer_count,
    };

    // SAFETY (whole function): forwarded contract.
    unsafe {
        // Transition base level to SRC_OPTIMAL for blitting.
        set_image_layout(
            fns.cmd_pipeline_barrier,
            vdi.cmd_buffer,
            vk_texture.image,
            initial_layout,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            subresource_range,
        );

        // Generate the mip chain
        // ----------------------
        // Blit level n from level n-1.
        let dim = |v: u32, s: u32| (v.checked_shr(s).unwrap_or(0)).max(1) as i32;
        for i in 1..vk_texture.level_count {
            let image_blit = vk::ImageBlit {
                // Source
                src_subresource: vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    layer_count: vk_texture.layer_count,
                    mip_level: i - 1,
                    base_array_layer: 0,
                },
                src_offsets: [
                    vk::Offset3D::default(),
                    vk::Offset3D {
                        x: dim(vk_texture.width, i - 1),
                        y: dim(vk_texture.height, i - 1),
                        z: dim(vk_texture.depth, i - 1),
                    },
                ],
                // Destination
                dst_subresource: vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    layer_count: 1,
                    mip_level: i,
                    base_array_layer: 0,
                },
                dst_offsets: [
                    vk::Offset3D::default(),
                    vk::Offset3D {
                        x: dim(vk_texture.width, i),
                        y: dim(vk_texture.height, i),
                        z: dim(vk_texture.depth, i),
                    },
                ],
            };

            let mip_sub_range = vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: i,
                level_count: 1,
                base_array_layer: 0,
                layer_count: vk_texture.layer_count,
            };

            // Transiton current mip level to transfer dest
            set_image_layout(
                fns.cmd_pipeline_barrier,
                vdi.cmd_buffer,
                vk_texture.image,
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                mip_sub_range,
            );

            // Blit from previous level
            (fns.cmd_blit_image)(
                vdi.cmd_buffer,
                vk_texture.image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                vk_texture.image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                1,
                &image_blit,
                blit_filter,
            );

            // Transiton current mip level to transfer source for read in next iteration.
            set_image_layout(
                fns.cmd_pipeline_barrier,
                vdi.cmd_buffer,
                vk_texture.image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                mip_sub_range,
            );
        }

        // After the loop, all mip layers are in TRANSFER_SRC layout. Transition all to final
        // layout.
        subresource_range.level_count = vk_texture.level_count;
        set_image_layout(
            fns.cmd_pipeline_barrier,
            vdi.cmd_buffer,
            vk_texture.image,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            vk_texture.image_layout,
            subresource_range,
        );
    }
}

/// Port of `ktxTexture_VkUploadEx_WithSuballocator`: creates a Vulkan image on the device of
/// `vdi` holding the texture's images (optimal tiling via a staging buffer, or a linear image
/// written through mapped memory), optionally generating the mip levels with blits.
///
/// # Safety
/// `vdi` must have been constructed with valid handles that are still alive; the queue and
/// `vdi.cmd_buffer` must not be in use elsewhere. The suballocator callbacks must implement
/// their contract on the same device.
pub unsafe fn ktx_texture_vk_upload_ex_with_suballocator(
    this: &mut dyn KtxTexture,
    vdi: &mut VulkanDeviceInfo,
    vk_texture: &mut VulkanTexture,
    tiling: vk::ImageTiling,
    mut usage_flags: vk::ImageUsageFlags,
    final_layout: vk::ImageLayout,
    sub_allocator_callbacks: Option<&SubAllocatorCallbacks>,
) -> KtxResult<()> {
    let mut blit_filter = vk::Filter::LINEAR;
    let mut create_flags = vk::ImageCreateFlags::empty();
    let element_size = this.base().get_element_size();

    let sub = match sub_allocator_callbacks {
        Some(cb) => Some(cb.complete()?),
        None => None,
    };
    let fns = UploadFns::new(&vdi.vk_funcs)?;

    let t = this.base();
    if t.p_data.is_none() && !t.is_active_stream() {
        // Nothing to upload.
        return Err(ErrorCode::InvalidOperation);
    }

    let mut num_image_layers = t.num_layers;
    if t.is_cubemap {
        num_image_layers *= 6;
        create_flags = vk::ImageCreateFlags::CUBE_COMPATIBLE;
    }

    let (image_type, view_type) = match t.num_dimensions {
        1 => (
            vk::ImageType::TYPE_1D,
            if t.is_array {
                vk::ImageViewType::TYPE_1D_ARRAY
            } else {
                vk::ImageViewType::TYPE_1D
            },
        ),
        3 => (vk::ImageType::TYPE_3D, vk::ImageViewType::TYPE_3D),
        // 2 and default: To keep compilers happy.
        _ => (
            vk::ImageType::TYPE_2D,
            match (t.is_cubemap, t.is_array) {
                (true, true) => vk::ImageViewType::CUBE_ARRAY,
                (true, false) => vk::ImageViewType::CUBE,
                (false, true) => vk::ImageViewType::TYPE_2D_ARRAY,
                (false, false) => vk::ImageViewType::TYPE_2D,
            },
        ),
    };

    let vk_format = vk::Format::from_raw(this.get_vk_format().0 as i32);
    if vk_format == vk::Format::UNDEFINED {
        return Err(ErrorCode::InvalidOperation);
    }

    // Get device properties for the requested image format
    if tiling == vk::ImageTiling::OPTIMAL {
        // Ensure we can copy from staging buffer to image.
        usage_flags |= vk::ImageUsageFlags::TRANSFER_DST;
    }
    if t.generate_mipmaps {
        // Ensure we can blit between levels.
        usage_flags |= vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::TRANSFER_SRC;
    }
    let mut image_format_properties = vk::ImageFormatProperties::default();
    // SAFETY: valid physical device; plain output struct.
    let v_result = unsafe {
        (fns.get_physical_device_image_format_properties)(
            vdi.physical_device,
            vk_format,
            image_type,
            tiling,
            usage_flags,
            create_flags,
            &mut image_format_properties,
        )
    };
    if v_result == vk::Result::ERROR_FORMAT_NOT_SUPPORTED {
        return Err(ErrorCode::InvalidOperation);
    }
    if t.num_layers > image_format_properties.max_array_layers {
        return Err(ErrorCode::InvalidOperation);
    }

    let num_image_levels = if t.generate_mipmaps {
        let needed_features = vk::FormatFeatureFlags::BLIT_DST | vk::FormatFeatureFlags::BLIT_SRC;
        let mut format_properties = vk::FormatProperties::default();
        // SAFETY: valid physical device; plain output struct.
        unsafe {
            (fns.get_physical_device_format_properties)(
                vdi.physical_device,
                vk_format,
                &mut format_properties,
            )
        };
        let format_feature_flags = if tiling == vk::ImageTiling::OPTIMAL {
            format_properties.optimal_tiling_features
        } else {
            format_properties.linear_tiling_features
        };
        if !format_feature_flags.contains(needed_features) {
            return Err(ErrorCode::InvalidOperation);
        }
        blit_filter =
            if format_feature_flags.contains(vk::FormatFeatureFlags::SAMPLED_IMAGE_FILTER_LINEAR) {
                vk::Filter::LINEAR
            } else {
                vk::Filter::NEAREST // XXX INVALID_OP?
            };
        let max_dim = t.base_width.max(t.base_height).max(t.base_depth);
        f64::from(max_dim).log2().floor() as u32 + 1
    } else {
        t.num_levels
    };

    if num_image_levels > image_format_properties.max_mip_levels {
        return Err(ErrorCode::InvalidOperation);
    }

    let can_use_faster_path = if t.class_id == ClassId::Texture2 {
        true
    } else {
        let actual_row_pitch = t.get_row_pitch(0);
        let tight_row_pitch = element_size * t.base_width;
        // If the texture's images do not have any row padding, we can use a faster path. Only
        // uncompressed textures might have padding.
        //
        // The first test in the if will match compressed textures, because they all have a
        // block size that is a multiple of 4, as well as a class of uncompressed textures that
        // will never need padding.
        //
        // The second test matches textures whose level 0 has no padding. Any texture whose
        // block size is not a multiple of 4 will need padding at some miplevel even if level 0
        // does not. So, if more than 1 level exists, we must use the slower path.
        //
        // Note all elementSizes > 4 Will be a multiple of 4, so only elementSizes of 1, 2 & 3
        // are a concern here.
        element_size.is_multiple_of(4) // There'll be no padding at any level.
            // There is no padding at level 0 and no other levels.
            || (t.num_levels == 1 && actual_row_pitch == tight_row_pitch)
    };

    vk_texture.width = t.base_width;
    vk_texture.height = t.base_height;
    vk_texture.depth = t.base_depth;
    vk_texture.image_layout = final_layout;
    vk_texture.image_format = vk_format;
    vk_texture.level_count = num_image_levels;
    vk_texture.layer_count = num_image_layers;
    vk_texture.view_type = view_type;
    vk_texture.vk_destroy_image = vdi.vk_funcs.vk_destroy_image;
    vk_texture.vk_free_memory = vdi.vk_funcs.vk_free_memory;

    let (num_layers, num_faces, num_levels, num_dimensions, is_array, gen_mipmaps) = (
        t.num_layers,
        t.num_faces,
        t.num_levels,
        t.num_dimensions,
        t.is_array,
        t.generate_mipmaps,
    );
    let extent = vk::Extent3D {
        width: vk_texture.width,
        height: vk_texture.height,
        depth: vk_texture.depth,
    };

    let cmd_buf_begin_info = vk::CommandBufferBeginInfo::default();
    // SAFETY (whole upload): the device objects come from `vdi` (caller contract); every
    // pointer passed points to a local that outlives the call; mapped memory is only accessed
    // through bounds checked `Mapped` slices while mapped.
    unsafe {
        let _ = (fns.begin_command_buffer)(vdi.cmd_buffer, &cmd_buf_begin_info);

        if tiling == vk::ImageTiling::OPTIMAL {
            // Create a host-visible staging buffer that contains the raw image data
            let texture_size = this.get_data_size_uncompressed() as u64;
            let mut buffer_create_info = vk::BufferCreateInfo {
                size: texture_size,
                ..Default::default()
            };
            let num_copy_regions;
            if can_use_faster_path {
                num_copy_regions = num_levels;
            } else {
                num_copy_regions = if is_array {
                    num_levels
                } else {
                    num_levels * num_faces
                };
                buffer_create_info.size += u64::from(num_copy_regions * element_size * 4);
            }

            // This buffer is used as a transfer source for the buffer copy
            buffer_create_info.usage = vk::BufferUsageFlags::TRANSFER_SRC;
            buffer_create_info.sharing_mode = vk::SharingMode::EXCLUSIVE;

            let mut staging_buffer = vk::Buffer::null();
            let _ = (fns.create_buffer)(
                vdi.device,
                &buffer_create_info,
                vdi.p_allocator,
                &mut staging_buffer,
            );

            // Get memory requirements for the staging buffer (alignment, memory type bits)
            let mut mem_reqs = vk::MemoryRequirements::default();
            (fns.get_buffer_memory_requirements)(vdi.device, staging_buffer, &mut mem_reqs);

            let mut mem_alloc_info = vk::MemoryAllocateInfo {
                allocation_size: mem_reqs.size,
                // Get memory type index for a host visible buffer
                memory_type_index: vdi.get_memory_type(
                    mem_reqs.memory_type_bits,
                    vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
                ),
                ..Default::default()
            };

            let mut staging_memory = vk::DeviceMemory::null();
            let mut staging_alloc_id = 0u64;
            let mut p_mapped: *mut c_void = std::ptr::null_mut();
            let mut mapped_len = mem_reqs.size;
            match &sub {
                None => {
                    let v_result = (fns.allocate_memory)(
                        vdi.device,
                        &mem_alloc_info,
                        vdi.p_allocator,
                        &mut staging_memory,
                    );
                    if v_result != vk::Result::SUCCESS {
                        return Err(ErrorCode::OutOfMemory);
                    }
                    let _ = (fns.bind_buffer_memory)(vdi.device, staging_buffer, staging_memory, 0);
                    let _ = (fns.map_memory)(
                        vdi.device,
                        staging_memory,
                        0,
                        mem_reqs.size,
                        vk::MemoryMapFlags::empty(),
                        &mut p_mapped,
                    );
                }
                Some(s) => {
                    let mut num_pages = 0u64;
                    staging_alloc_id = (s.alloc_mem)(&mem_alloc_info, &mem_reqs, &mut num_pages);
                    if staging_alloc_id == 0 {
                        return Err(ErrorCode::OutOfMemory);
                    }
                    if num_pages > 1 {
                        // Sparse binding of KTX textures is unsupported for the moment
                        return Err(ErrorCode::UnsupportedFeature);
                    }
                    let _ = (s.bind_buffer)(staging_buffer, staging_alloc_id);
                    let _ = (s.memory_map)(staging_alloc_id, 0, &mut mapped_len, &mut p_mapped);
                }
            }
            let mut mapped = Mapped {
                ptr: p_mapped.cast::<u8>(),
                len: usize::try_from(mapped_len).unwrap_or(0),
            };

            let mut cb_data = OptimalCbData {
                regions: Vec::with_capacity(num_copy_regions as usize),
                offset: 0,
                num_faces,
                num_layers,
                dest: &mut mapped,
                element_size,
                num_dimensions,
            };

            let k_result: KtxResult<()> = if can_use_faster_path {
                // Bulk load the data to the staging buffer and iterate over levels.
                let loaded = match this.base().p_data.as_deref() {
                    // Image data has already been loaded. Copy to staging buffer.
                    Some(data) => cb_data.dest.write(0, data),
                    // Load the image data directly into the staging buffer.
                    None => {
                        let len =
                            usize::try_from(mem_alloc_info.allocation_size).unwrap_or(usize::MAX);
                        let dst = cb_data.dest.slice();
                        let n = len.min(dst.len());
                        this.load_image_data(dst.get_mut(..n))
                    }
                };
                loaded?;
                // Iterate over mip levels to set up the copy regions.
                // XXX Check for possible errors.
                this.iterate_levels(&mut |miplevel, face, w, h, d, size, _| {
                    cb_data.optimal_tiling(miplevel, face, w, h, d, size);
                    Ok(())
                })
            } else {
                // Iterate over face-levels with callback that copies the face-levels to
                // Vulkan-valid offsets in the staging buffer while removing padding. Using
                // face-levels minimizes pre-staging-buffer buffering, in the event the data is
                // not already loaded.
                let mut cb = |miplevel, face, w, h, d, size, pixels: &[u8]| {
                    cb_data.optimal_tiling_pad(miplevel, face, w, h, d, size, pixels)
                };
                if this.base().p_data.is_some() {
                    this.iterate_level_faces(&mut cb)
                } else {
                    // XXX Check for possible errors.
                    this.iterate_load_level_faces(&mut cb)
                }
            };
            let copy_regions = std::mem::take(&mut cb_data.regions);
            drop(cb_data);

            match &sub {
                None => (fns.unmap_memory)(vdi.device, staging_memory),
                Some(s) => (s.memory_unmap)(staging_alloc_id, 0),
            }
            // A failed copy into the bounded mapping would read past the source in C.
            k_result?;

            // Create optimal tiled target image
            let image_create_info = vk::ImageCreateInfo {
                image_type,
                flags: create_flags,
                format: vk_format,
                // numImageLevels ensures enough levels for generateMipmaps.
                mip_levels: num_image_levels,
                array_layers: num_image_layers,
                samples: vk::SampleCountFlags::TYPE_1,
                tiling: vk::ImageTiling::OPTIMAL,
                usage: usage_flags,
                sharing_mode: vk::SharingMode::EXCLUSIVE,
                initial_layout: vk::ImageLayout::UNDEFINED,
                extent,
                ..Default::default()
            };
            let _ = (fns.create_image)(
                vdi.device,
                &image_create_info,
                vdi.p_allocator,
                &mut vk_texture.image,
            );

            (fns.get_image_memory_requirements)(vdi.device, vk_texture.image, &mut mem_reqs);

            mem_alloc_info.allocation_size = mem_reqs.size;
            mem_alloc_info.memory_type_index = vdi.get_memory_type(
                mem_reqs.memory_type_bits,
                vk::MemoryPropertyFlags::DEVICE_LOCAL,
            );
            match &sub {
                None => {
                    let _ = (fns.allocate_memory)(
                        vdi.device,
                        &mem_alloc_info,
                        vdi.p_allocator,
                        &mut vk_texture.device_memory,
                    );
                    let _ = (fns.bind_image_memory)(
                        vdi.device,
                        vk_texture.image,
                        vk_texture.device_memory,
                        0,
                    );
                }
                Some(s) => {
                    let mut num_pages = 0u64;
                    vk_texture.allocation_id =
                        (s.alloc_mem)(&mem_alloc_info, &mem_reqs, &mut num_pages);
                    if vk_texture.allocation_id == 0 {
                        return Err(ErrorCode::OutOfMemory);
                    }
                    if num_pages > 1 {
                        // Sparse binding of KTX textures is unsupported for the moment
                        return Err(ErrorCode::UnsupportedFeature);
                    }
                    let _ = (s.bind_image)(vk_texture.image, vk_texture.allocation_id);
                }
            }

            let subresource_range = vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: num_levels,
                base_array_layer: 0,
                layer_count: num_image_layers,
            };

            // Image barrier to transition, possibly only the base level, image layout to
            // TRANSFER_DST_OPTIMAL so it can be used as the copy destination.
            set_image_layout(
                fns.cmd_pipeline_barrier,
                vdi.cmd_buffer,
                vk_texture.image,
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                subresource_range,
            );

            // Copy mip levels from staging buffer
            (fns.cmd_copy_buffer_to_image)(
                vdi.cmd_buffer,
                staging_buffer,
                vk_texture.image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                copy_regions.len() as u32,
                copy_regions.as_ptr(),
            );

            if gen_mipmaps {
                generate_mipmaps(
                    &fns,
                    vk_texture,
                    vdi,
                    blit_filter,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                );
            } else {
                // Transition image layout to finalLayout after all mip levels have been copied.
                // In this case numImageLevels == This->numLevels
                set_image_layout(
                    fns.cmd_pipeline_barrier,
                    vdi.cmd_buffer,
                    vk_texture.image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    final_layout,
                    subresource_range,
                );
            }

            // Submit command buffer containing copy and image layout commands
            let _ = (fns.end_command_buffer)(vdi.cmd_buffer);

            // Create a fence to make sure that the copies have finished before continuing
            let fence_create_info = vk::FenceCreateInfo::default();
            let mut copy_fence = vk::Fence::null();
            let _ = (fns.create_fence)(
                vdi.device,
                &fence_create_info,
                vdi.p_allocator,
                &mut copy_fence,
            );

            let submit_info = vk::SubmitInfo {
                command_buffer_count: 1,
                p_command_buffers: &vdi.cmd_buffer,
                ..Default::default()
            };

            let _ = (fns.queue_submit)(vdi.queue, 1, &submit_info, copy_fence);
            let _ =
                (fns.wait_for_fences)(vdi.device, 1, &copy_fence, vk::TRUE, DEFAULT_FENCE_TIMEOUT);
            (fns.destroy_fence)(vdi.device, copy_fence, vdi.p_allocator);

            // Clean up staging resources
            match &sub {
                None => (fns.free_memory)(vdi.device, staging_memory, vdi.p_allocator),
                Some(s) => (s.free_mem)(staging_alloc_id),
            }
            (fns.destroy_buffer)(vdi.device, staging_buffer, vdi.p_allocator);
        } else {
            let image_create_info = vk::ImageCreateInfo {
                image_type,
                flags: create_flags,
                format: vk_format,
                extent,
                // numImageLevels ensures enough levels for generateMipmaps.
                mip_levels: num_image_levels,
                array_layers: num_image_layers,
                samples: vk::SampleCountFlags::TYPE_1,
                tiling: vk::ImageTiling::LINEAR,
                usage: usage_flags,
                sharing_mode: vk::SharingMode::EXCLUSIVE,
                initial_layout: vk::ImageLayout::PREINITIALIZED,
                ..Default::default()
            };

            // Load mip map level 0 to linear tiling image
            let mut mappable_image = vk::Image::null();
            let _ = (fns.create_image)(
                vdi.device,
                &image_create_info,
                vdi.p_allocator,
                &mut mappable_image,
            );

            // Get memory requirements for this image like size and alignment
            let mut mem_reqs = vk::MemoryRequirements::default();
            (fns.get_image_memory_requirements)(vdi.device, mappable_image, &mut mem_reqs);
            let mem_alloc_info = vk::MemoryAllocateInfo {
                // Set memory allocation size to required memory size
                allocation_size: mem_reqs.size,
                // Get memory type that can be mapped to host memory
                memory_type_index: vdi.get_memory_type(
                    mem_reqs.memory_type_bits,
                    vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
                ),
                ..Default::default()
            };

            // Allocate host memory
            let mut mappable_memory = vk::DeviceMemory::null();
            match &sub {
                None => {
                    let v_result = (fns.allocate_memory)(
                        vdi.device,
                        &mem_alloc_info,
                        vdi.p_allocator,
                        &mut mappable_memory,
                    );
                    if v_result != vk::Result::SUCCESS {
                        return Err(ErrorCode::OutOfMemory);
                    }
                    let _ = (fns.bind_image_memory)(vdi.device, mappable_image, mappable_memory, 0);
                }
                Some(s) => {
                    let mut num_pages = 0u64;
                    vk_texture.allocation_id =
                        (s.alloc_mem)(&mem_alloc_info, &mem_reqs, &mut num_pages);
                    if vk_texture.allocation_id == 0 {
                        return Err(ErrorCode::OutOfMemory);
                    }
                    if num_pages > 1 {
                        // Sparse binding of KTX textures is unsupported for the moment
                        return Err(ErrorCode::UnsupportedFeature);
                    }
                    let _ = (s.bind_image)(mappable_image, vk_texture.allocation_id);
                }
            }

            // Map image memory
            let mut p_mapped: *mut c_void = std::ptr::null_mut();
            let mut mapped_len = mem_reqs.size;
            match &sub {
                None => {
                    let _ = (fns.map_memory)(
                        vdi.device,
                        mappable_memory,
                        0,
                        mem_reqs.size,
                        vk::MemoryMapFlags::empty(),
                        &mut p_mapped,
                    );
                }
                Some(s) => {
                    let _ =
                        (s.memory_map)(vk_texture.allocation_id, 0, &mut mapped_len, &mut p_mapped);
                }
            }
            let mut mapped = Mapped {
                ptr: p_mapped.cast::<u8>(),
                len: usize::try_from(mapped_len).unwrap_or(0),
            };

            let levels = (0..num_levels)
                .map(|l| LevelInfo {
                    row_pitch: this.base().get_row_pitch(l),
                    image_size: this.get_image_size(l) as u64,
                })
                .collect();
            let mut cb_data = LinearCbData {
                get_image_subresource_layout: fns.get_image_subresource_layout,
                dest_image: mappable_image,
                device: vdi.device,
                dest: &mut mapped,
                levels,
                num_layers,
                num_faces,
                num_dimensions,
                pad: !can_use_faster_path,
            };
            let mut cb = |miplevel, face, _w, h, d, size, pixels: &[u8]| {
                cb_data.callback(miplevel, face, h, d, size, pixels)
            };

            // Iterate over images to copy texture data into mapped image memory.
            // XXX Check for possible errors
            let k_result = if this.base().is_active_stream() {
                this.iterate_load_level_faces(&mut cb)
            } else {
                this.iterate_level_faces(&mut cb)
            };

            match &sub {
                None => (fns.unmap_memory)(vdi.device, mappable_memory),
                Some(s) => (s.memory_unmap)(vk_texture.allocation_id, 0),
            }
            // A failed copy into the bounded mapping would read past the source in C.
            k_result?;

            // Linear tiled images can be directly used as textures.
            vk_texture.image = mappable_image;
            if sub.is_none() {
                vk_texture.device_memory = mappable_memory;
            }

            if gen_mipmaps {
                generate_mipmaps(
                    &fns,
                    vk_texture,
                    vdi,
                    blit_filter,
                    vk::ImageLayout::PREINITIALIZED,
                );
            } else {
                let subresource_range = vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: num_image_levels,
                    base_array_layer: 0,
                    layer_count: num_image_layers,
                };
                // Transition image layout to finalLayout.
                set_image_layout(
                    fns.cmd_pipeline_barrier,
                    vdi.cmd_buffer,
                    vk_texture.image,
                    vk::ImageLayout::PREINITIALIZED,
                    final_layout,
                    subresource_range,
                );
            }

            // Submit command buffer containing image layout commands
            let _ = (fns.end_command_buffer)(vdi.cmd_buffer);

            let submit_info = vk::SubmitInfo {
                wait_semaphore_count: 0,
                command_buffer_count: 1,
                p_command_buffers: &vdi.cmd_buffer,
                ..Default::default()
            };
            let _ = (fns.queue_submit)(vdi.queue, 1, &submit_info, vk::Fence::null());
            let _ = (fns.queue_wait_idle)(vdi.queue);
        }
    }
    Ok(())
}

/// Port of `ktxTexture_VkUploadEx`.
///
/// # Safety
/// See [`ktx_texture_vk_upload_ex_with_suballocator`].
pub unsafe fn ktx_texture_vk_upload_ex(
    this: &mut dyn KtxTexture,
    vdi: &mut VulkanDeviceInfo,
    vk_texture: &mut VulkanTexture,
    tiling: vk::ImageTiling,
    usage_flags: vk::ImageUsageFlags,
    final_layout: vk::ImageLayout,
) -> KtxResult<()> {
    // SAFETY: forwarded contract.
    unsafe {
        ktx_texture_vk_upload_ex_with_suballocator(
            this,
            vdi,
            vk_texture,
            tiling,
            usage_flags,
            final_layout,
            None,
        )
    }
}

/// Port of `ktxTexture_VkUpload`: optimal tiling, sampled, `SHADER_READ_ONLY_OPTIMAL`.
///
/// # Safety
/// See [`ktx_texture_vk_upload_ex_with_suballocator`].
pub unsafe fn ktx_texture_vk_upload(
    this: &mut dyn KtxTexture,
    vdi: &mut VulkanDeviceInfo,
    vk_texture: &mut VulkanTexture,
) -> KtxResult<()> {
    // SAFETY: forwarded contract.
    unsafe {
        ktx_texture_vk_upload_ex(
            this,
            vdi,
            vk_texture,
            vk::ImageTiling::OPTIMAL,
            vk::ImageUsageFlags::SAMPLED,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        )
    }
}

/// `ktxTexture1_VkUpload*` / `ktxTexture2_VkUpload*`: the upload functions as methods.
macro_rules! vk_upload_methods {
    ($t:ty, $c:literal) => {
        impl $t {
            #[doc = concat!("Port of `", $c, "_VkUploadEx_WithSuballocator`.")]
            ///
            /// # Safety
            /// See [`ktx_texture_vk_upload_ex_with_suballocator`].
            pub unsafe fn vk_upload_ex_with_suballocator(
                &mut self,
                vdi: &mut VulkanDeviceInfo,
                vk_texture: &mut VulkanTexture,
                tiling: vk::ImageTiling,
                usage_flags: vk::ImageUsageFlags,
                final_layout: vk::ImageLayout,
                sub_allocator_callbacks: Option<&SubAllocatorCallbacks>,
            ) -> KtxResult<()> {
                // SAFETY: forwarded contract.
                unsafe {
                    ktx_texture_vk_upload_ex_with_suballocator(
                        self,
                        vdi,
                        vk_texture,
                        tiling,
                        usage_flags,
                        final_layout,
                        sub_allocator_callbacks,
                    )
                }
            }

            #[doc = concat!("Port of `", $c, "_VkUploadEx`.")]
            ///
            /// # Safety
            /// See [`ktx_texture_vk_upload_ex_with_suballocator`].
            pub unsafe fn vk_upload_ex(
                &mut self,
                vdi: &mut VulkanDeviceInfo,
                vk_texture: &mut VulkanTexture,
                tiling: vk::ImageTiling,
                usage_flags: vk::ImageUsageFlags,
                final_layout: vk::ImageLayout,
            ) -> KtxResult<()> {
                // SAFETY: forwarded contract.
                unsafe {
                    ktx_texture_vk_upload_ex(
                        self,
                        vdi,
                        vk_texture,
                        tiling,
                        usage_flags,
                        final_layout,
                    )
                }
            }

            #[doc = concat!("Port of `", $c, "_VkUpload`.")]
            ///
            /// # Safety
            /// See [`ktx_texture_vk_upload_ex_with_suballocator`].
            pub unsafe fn vk_upload(
                &mut self,
                vdi: &mut VulkanDeviceInfo,
                vk_texture: &mut VulkanTexture,
            ) -> KtxResult<()> {
                // SAFETY: forwarded contract.
                unsafe { ktx_texture_vk_upload(self, vdi, vk_texture) }
            }
        }
    };
}

vk_upload_methods!(Texture1, "ktxTexture1");
vk_upload_methods!(Texture2, "ktxTexture2");
vk_upload_methods!(AnyTexture, "ktxTexture");

impl VulkanTexture {
    /// Port of `ktxVulkanTexture_Destruct_WithSuballocator`: destroys the image and frees its
    /// memory (through the suballocator when given).
    ///
    /// # Safety
    /// `device` must be the device the texture was created on and the image no longer in use.
    pub unsafe fn destruct_with_suballocator(
        &mut self,
        device: vk::Device,
        p_allocator: *const vk::AllocationCallbacks<'static>,
        sub_allocator_callbacks: Option<&SubAllocatorCallbacks>,
    ) -> KtxResult<()> {
        let sub = match sub_allocator_callbacks {
            Some(cb) => Some(cb.complete()?),
            None => None,
        };
        // SAFETY: forwarded contract.
        unsafe {
            if let Some(destroy_image) = self.vk_destroy_image {
                destroy_image(device, self.image, p_allocator);
            }
            match sub {
                Some(s) => (s.free_mem)(self.allocation_id),
                None => {
                    if let Some(free_memory) = self.vk_free_memory {
                        free_memory(device, self.device_memory, p_allocator);
                    }
                }
            }
        }
        Ok(())
    }

    /// Port of `ktxVulkanTexture_Destruct`.
    ///
    /// # Safety
    /// See [`VulkanTexture::destruct_with_suballocator`].
    pub unsafe fn destruct(
        &mut self,
        device: vk::Device,
        p_allocator: *const vk::AllocationCallbacks<'static>,
    ) {
        // SAFETY: forwarded contract.
        let _ = unsafe { self.destruct_with_suballocator(device, p_allocator, None) };
    }
}
