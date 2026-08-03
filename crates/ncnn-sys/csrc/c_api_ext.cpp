// GPU enumeration for ncnn's C API.
//
// ncnn's C API covers the whole inference path but stops at device discovery:
// get_gpu_count(), get_default_gpu_index() and the GpuInfo accessors are C++
// only, while selection (ncnn_net_set_vulkan_device) is exposed. That gap is
// the one thing this project cannot live without -- mesa-vulkan-drivers
// installs lavapipe beside RADV, so a software rasterizer always enumerates
// next to the real GPU, and falling onto it silently looks like a working
// Vulkan path at a fraction of the speed.
//
// This is a translation unit of ours compiled against ncnn's installed
// headers, not a patch to ncnn: nothing here needs a fork or a vendored tree.
#include "c_api_ext.h"

#include <platform.h>

#if NCNN_VULKAN
#include <gpu.h>

// get_gpu_info() indexes an array without bounds-checking, so every entry
// point validates first and reports the miss rather than reading past it.
static bool ncnn_ext_valid(int device_index)
{
    return device_index >= 0 && device_index < ncnn::get_gpu_count();
}
#endif

extern "C" {

int ncnn_ext_get_gpu_count(void)
{
#if NCNN_VULKAN
    // Creates the Vulkan instance on first call and returns 0 if that fails,
    // which is also how a host with no usable ICD reports itself.
    return ncnn::get_gpu_count();
#else
    return 0;
#endif
}

int ncnn_ext_get_default_gpu_index(void)
{
#if NCNN_VULKAN
    if (ncnn::get_gpu_count() <= 0)
        return -1;
    return ncnn::get_default_gpu_index();
#else
    return -1;
#endif
}

int ncnn_ext_get_gpu_type(int device_index)
{
#if NCNN_VULKAN
    if (!ncnn_ext_valid(device_index))
        return -1;
    return ncnn::get_gpu_info(device_index).type();
#else
    (void)device_index;
    return -1;
#endif
}

const char* ncnn_ext_get_gpu_device_name(int device_index)
{
#if NCNN_VULKAN
    if (!ncnn_ext_valid(device_index))
        return 0;
    return ncnn::get_gpu_info(device_index).device_name();
#else
    (void)device_index;
    return 0;
#endif
}

const char* ncnn_ext_get_gpu_driver_name(int device_index)
{
#if NCNN_VULKAN
    if (!ncnn_ext_valid(device_index))
        return 0;
    return ncnn::get_gpu_info(device_index).driver_name();
#else
    (void)device_index;
    return 0;
#endif
}

unsigned int ncnn_ext_get_gpu_rough_score(int device_index)
{
#if NCNN_VULKAN
    if (!ncnn_ext_valid(device_index))
        return 0;
    return ncnn::get_gpu_info(device_index).rough_score();
#else
    (void)device_index;
    return 0;
#endif
}

void ncnn_ext_destroy_gpu_instance(void)
{
#if NCNN_VULKAN
    ncnn::destroy_gpu_instance();
#endif
}

} // extern "C"
