//! The painter through the C ABI, in a binary of its own: making its Vulkan device loads the
//! machine's Vulkan drivers (on a runner, Mesa's, each with LLVM), which held every other thread of
//! the process for 2 to 8 seconds there (the crate's quick tests waiting together on a 4-core arm
//! runner, a request's five seconds running out with no answer at all). Among the crate's own
//! tests it stalled whichever ran beside it.

#![allow(unsafe_code)]

/// A painter through the C ABI: its Vulkan objects as Skia is given them, and a present without a
/// client refused; skipped without a Vulkan device. Presenting itself is `oxyos-paint`'s test.
#[test]
fn a_painter_gives_its_vulkan_objects_through_the_c_abi() {
    use oxyos_client_ffi::paint::*;
    use oxyos_client_ffi::{OxyOSRect, OXYOS_ERROR, OXYOS_FORMAT_XRGB8888};
    use std::ffi::CStr;
    // SAFETY: every pointer below is live for its call; the painter is freed once.
    unsafe {
        let paint = oxyos_paint_new(0);
        if paint.is_null() {
            eprintln!("no Vulkan device, skipped");
            return;
        }
        assert!(!oxyos_paint_device_name(paint).is_null());
        assert_eq!(oxyos_paint_vulkan(paint, std::ptr::null_mut()), OXYOS_ERROR);
        let mut out = std::mem::MaybeUninit::<OxyOSPaintVulkan>::zeroed();
        assert_eq!(oxyos_paint_vulkan(paint, out.as_mut_ptr()), 0);
        let vulkan = out.assume_init();
        assert!(!vulkan.get_instance_proc_addr.is_null());
        assert!(vulkan.instance != 0 && vulkan.physical_device != 0);
        assert!(vulkan.device != 0 && vulkan.queue != 0);
        assert_eq!(vulkan.api_version, (1 << 22) | (1 << 12), "Vulkan 1.1");
        assert_eq!(vulkan.instance_extension_count, 0);
        let names: Vec<String> =
            std::slice::from_raw_parts(vulkan.device_extensions, vulkan.device_extension_count)
                .iter()
                .map(|&name| CStr::from_ptr(name).to_string_lossy().into_owned())
                .collect();
        // It presents only with DMA-BUF export enabled, and names what it enabled.
        let dmabuf = names
            .iter()
            .any(|name| name == "VK_EXT_external_memory_dma_buf");
        assert!(!oxyos_paint_exports(paint) || dmabuf);
        // VkPhysicalDeviceFeatures2: sType, pNext (none), then robustBufferAccess, disabled.
        let features = vulkan.features.cast::<u8>();
        assert_eq!(
            features.cast::<u32>().read(),
            1_000_059_000,
            "VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2"
        );
        assert!(features.add(8).cast::<*const u8>().read().is_null());
        assert_eq!(features.add(16).cast::<u32>().read(), 0);
        let rect = OxyOSRect {
            x: 0,
            y: 0,
            width: 4,
            height: 4,
        };
        assert_eq!(
            oxyos_paint_present(
                paint,
                std::ptr::null_mut(),
                1,
                1,
                0,
                4,
                4,
                OXYOS_FORMAT_XRGB8888,
                &rect,
                1
            ),
            -1,
            "no client"
        );
        oxyos_paint_free(paint);
    }
}
