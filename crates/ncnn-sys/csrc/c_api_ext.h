/* GPU enumeration for ncnn's C API. See c_api_ext.cpp. */
#ifndef NCNN_C_API_EXT_H
#define NCNN_C_API_EXT_H

#ifdef __cplusplus
extern "C" {
#endif

/* 0 when ncnn was built without Vulkan, or when no device enumerates. */
int ncnn_ext_get_gpu_count(void);

/* ncnn's own highest-scoring device, or -1 if there is none. */
int ncnn_ext_get_default_gpu_index(void);

/* Device type: 0 discrete, 1 integrated, 2 virtual, 3 cpu. -1 if out of range. */
int ncnn_ext_get_gpu_type(int device_index);

/* Owned by ncnn and valid for the life of the Vulkan instance. NULL if out of range. */
const char* ncnn_ext_get_gpu_device_name(int device_index);
const char* ncnn_ext_get_gpu_driver_name(int device_index);

/* ncnn's own device score -- what get_default_gpu_index() maximises. */
unsigned int ncnn_ext_get_gpu_rough_score(int device_index);

void ncnn_ext_destroy_gpu_instance(void);

#ifdef __cplusplus
}
#endif

#endif /* NCNN_C_API_EXT_H */
