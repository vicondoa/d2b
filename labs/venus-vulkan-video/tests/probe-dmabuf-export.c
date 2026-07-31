/* Probe: can Venus export a Vulkan-video decode-output image as a DMA-BUF?
 *
 * This answers the question left open by the RDD crash in vn_GetMemoryFdKHR,
 * where mem->base_bo was NULL. Venus only populates base_bo when the memory is
 * allocated through the export path, and ffmpeg only requests exportable memory
 * when BOTH of these hold (hwcontext_vulkan.c, vulkan_pool_alloc):
 *
 *     p->vkctx.extensions & FF_VK_EXT_EXTERNAL_DMABUF_MEMORY
 *     hwctx->tiling == VK_IMAGE_TILING_DRM_FORMAT_MODIFIER_EXT
 *
 * and then only for the handle types that try_export_flags() confirms via
 * vkGetPhysicalDeviceImageFormatProperties2. So the decisive question is not
 * "does Venus advertise VK_EXT_external_memory_dma_buf" -- it does -- but
 * whether that query reports EXPORTABLE for the exact decode-output image:
 * NV12, DRM-modifier tiling, VIDEO_DECODE_DST usage, with an H.264 decode
 * profile in the chain.
 *
 * The video profile list is required for any query carrying video usage bits;
 * omitting it makes the query invalid rather than merely unsupported, so this
 * probe runs the profile-carrying form as the real case and the profile-less
 * and OPTIMAL-tiling forms as controls. If the real case is unsupported while
 * a control succeeds, that names precisely what Venus is missing.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <vulkan/vulkan.h>

static const char *yn(int b) { return b ? "YES" : "no"; }

static void
print_extmem_features(VkExternalMemoryFeatureFlags f)
{
   printf("exportable=%s importable=%s dedicated_only=%s (0x%x)",
          yn(f & VK_EXTERNAL_MEMORY_FEATURE_EXPORTABLE_BIT),
          yn(f & VK_EXTERNAL_MEMORY_FEATURE_IMPORTABLE_BIT),
          yn(f & VK_EXTERNAL_MEMORY_FEATURE_DEDICATED_ONLY_BIT),
          (unsigned)f);
}

/* One query. Returns VkResult so the caller can distinguish
 * FORMAT_NOT_SUPPORTED (a clean "no") from anything stranger.
 */
static VkResult
query(VkPhysicalDevice dev, VkImageTiling tiling, uint64_t modifier,
      VkImageUsageFlags usage, int with_profile, int with_external,
      VkExternalMemoryFeatureFlags *out_features)
{
   VkVideoDecodeH264ProfileInfoKHR h264 = {
      .sType = VK_STRUCTURE_TYPE_VIDEO_DECODE_H264_PROFILE_INFO_KHR,
      .stdProfileIdc = 100 /* High */,
      .pictureLayout = VK_VIDEO_DECODE_H264_PICTURE_LAYOUT_PROGRESSIVE_KHR,
   };
   VkVideoProfileInfoKHR profile = {
      .sType = VK_STRUCTURE_TYPE_VIDEO_PROFILE_INFO_KHR,
      .pNext = &h264,
      .videoCodecOperation = VK_VIDEO_CODEC_OPERATION_DECODE_H264_BIT_KHR,
      .chromaSubsampling = VK_VIDEO_CHROMA_SUBSAMPLING_420_BIT_KHR,
      .lumaBitDepth = VK_VIDEO_COMPONENT_BIT_DEPTH_8_BIT_KHR,
      .chromaBitDepth = VK_VIDEO_COMPONENT_BIT_DEPTH_8_BIT_KHR,
   };
   VkVideoProfileListInfoKHR profile_list = {
      .sType = VK_STRUCTURE_TYPE_VIDEO_PROFILE_LIST_INFO_KHR,
      .profileCount = 1,
      .pProfiles = &profile,
   };

   /* Mirrors ffmpeg's chain order: ImageFormatInfo2 -> ExternalImageFormatInfo
    * -> ImageDrmFormatModifierInfoEXT, with the profile list appended last so
    * the video usage bits are legal.
    */
   VkPhysicalDeviceImageDrmFormatModifierInfoEXT mod_info = {
      .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_DRM_FORMAT_MODIFIER_INFO_EXT,
      .drmFormatModifier = modifier,
      .sharingMode = VK_SHARING_MODE_EXCLUSIVE,
      .pNext = with_profile ? (const void *)&profile_list : NULL,
   };
   VkPhysicalDeviceExternalImageFormatInfo ext_info = {
      .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_IMAGE_FORMAT_INFO,
      .handleType = VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT,
      .pNext = (tiling == VK_IMAGE_TILING_DRM_FORMAT_MODIFIER_EXT)
                  ? (const void *)&mod_info
                  : (with_profile ? (const void *)&profile_list : NULL),
   };

   const void *head;
   if (with_external)
      head = &ext_info;
   else if (tiling == VK_IMAGE_TILING_DRM_FORMAT_MODIFIER_EXT)
      head = &mod_info;
   else
      head = with_profile ? (const void *)&profile_list : NULL;

   VkPhysicalDeviceImageFormatInfo2 info = {
      .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_FORMAT_INFO_2,
      .pNext = head,
      .format = VK_FORMAT_G8_B8R8_2PLANE_420_UNORM,
      .type = VK_IMAGE_TYPE_2D,
      .tiling = tiling,
      .usage = usage,
      .flags = 0,
   };

   VkExternalImageFormatProperties ext_props = {
      .sType = VK_STRUCTURE_TYPE_EXTERNAL_IMAGE_FORMAT_PROPERTIES,
   };
   VkImageFormatProperties2 props = {
      .sType = VK_STRUCTURE_TYPE_IMAGE_FORMAT_PROPERTIES_2,
      .pNext = with_external ? &ext_props : NULL,
   };

   VkResult r = vkGetPhysicalDeviceImageFormatProperties2(dev, &info, &props);
   if (out_features)
      *out_features = ext_props.externalMemoryProperties.externalMemoryFeatures;
   return r;
}

static const char *
res_str(VkResult r)
{
   switch (r) {
   case VK_SUCCESS: return "VK_SUCCESS";
   case VK_ERROR_FORMAT_NOT_SUPPORTED: return "VK_ERROR_FORMAT_NOT_SUPPORTED";
   case VK_ERROR_OUT_OF_HOST_MEMORY: return "VK_ERROR_OUT_OF_HOST_MEMORY";
   case VK_ERROR_OUT_OF_DEVICE_MEMORY: return "VK_ERROR_OUT_OF_DEVICE_MEMORY";
   case VK_ERROR_INITIALIZATION_FAILED: return "VK_ERROR_INITIALIZATION_FAILED";
   default: break;
   }
   static char buf[32];
   snprintf(buf, sizeof(buf), "VkResult(%d)", (int)r);
   return buf;
}

int
main(void)
{
   VkApplicationInfo app = {
      .sType = VK_STRUCTURE_TYPE_APPLICATION_INFO,
      .apiVersion = VK_API_VERSION_1_3,
   };
   VkInstanceCreateInfo ici = {
      .sType = VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO,
      .pApplicationInfo = &app,
   };
   VkInstance inst;
   if (vkCreateInstance(&ici, NULL, &inst) != VK_SUCCESS) {
      printf("vkCreateInstance failed\n");
      return 1;
   }

   uint32_t n = 0;
   vkEnumeratePhysicalDevices(inst, &n, NULL);
   VkPhysicalDevice *devs = calloc(n, sizeof(*devs));
   vkEnumeratePhysicalDevices(inst, &n, devs);

   int found = 0;
   /* Two passes. The guest has an llvmpipe device alongside Venus, so prefer
    * the Venus/NVIDIA one there; on the host the same binary must select the
    * bare NVIDIA device instead. Running the identical probe on both stacks is
    * the whole point -- it is what distinguishes "Venus does not forward this"
    * from "the NVIDIA driver does not support it", and those have completely
    * different remedies.
    */
   for (int pass = 0; pass < 2 && !found; pass++) {
      for (uint32_t d = 0; d < n; d++) {
         VkPhysicalDeviceProperties props;
         vkGetPhysicalDeviceProperties(devs[d], &props);
         if (pass == 0) {
            if (!strstr(props.deviceName, "Venus") ||
                !strstr(props.deviceName, "NVIDIA"))
               continue;
         } else {
            if (props.deviceType == VK_PHYSICAL_DEVICE_TYPE_CPU)
               continue;
         }
         found = 1;
         printf("device: %s\n\n", props.deviceName);

      /* Modifiers Venus reports for NV12, and which carry decode-output. */
      VkDrmFormatModifierPropertiesListEXT list = {
         .sType = VK_STRUCTURE_TYPE_DRM_FORMAT_MODIFIER_PROPERTIES_LIST_EXT,
      };
      VkFormatProperties2 fp = {
         .sType = VK_STRUCTURE_TYPE_FORMAT_PROPERTIES_2,
         .pNext = &list,
      };
      vkGetPhysicalDeviceFormatProperties2(
         devs[d], VK_FORMAT_G8_B8R8_2PLANE_420_UNORM, &fp);

      VkDrmFormatModifierPropertiesEXT *mods =
         calloc(list.drmFormatModifierCount, sizeof(*mods));
      list.pDrmFormatModifierProperties = mods;
      vkGetPhysicalDeviceFormatProperties2(
         devs[d], VK_FORMAT_G8_B8R8_2PLANE_420_UNORM, &fp);

      printf("NV12 DRM format modifiers: %u\n", list.drmFormatModifierCount);
      for (uint32_t i = 0; i < list.drmFormatModifierCount; i++) {
         VkFormatFeatureFlags f = mods[i].drmFormatModifierTilingFeatures;
         printf("  mod 0x%016llx planes=%u decode_output=%s sampled=%s\n",
                (unsigned long long)mods[i].drmFormatModifier,
                mods[i].drmFormatModifierPlaneCount,
                yn(f & VK_FORMAT_FEATURE_VIDEO_DECODE_OUTPUT_BIT_KHR),
                yn(f & VK_FORMAT_FEATURE_SAMPLED_IMAGE_BIT));
      }
      printf("\n");

      const VkImageUsageFlags decode_usage =
         VK_IMAGE_USAGE_VIDEO_DECODE_DST_BIT_KHR |
         VK_IMAGE_USAGE_SAMPLED_BIT;

      /* Control A: OPTIMAL tiling, decode usage, no external. Establishes that
       * the decode image itself is supported at all.
       */
      VkResult r = query(devs[d], VK_IMAGE_TILING_OPTIMAL, 0, decode_usage,
                         1 /*profile*/, 0 /*external*/, NULL);
      printf("A optimal  + decode usage + profile, no external : %s\n",
             res_str(r));

      /* Control B: OPTIMAL tiling with DMA_BUF external. Vulkan does not
       * require modifier tiling for dma-buf export in general.
       */
      VkExternalMemoryFeatureFlags feat = 0;
      r = query(devs[d], VK_IMAGE_TILING_OPTIMAL, 0, decode_usage, 1, 1, &feat);
      printf("B optimal  + decode usage + profile + DMA_BUF   : %s", res_str(r));
      if (r == VK_SUCCESS) { printf("  "); print_extmem_features(feat); }
      printf("\n");

      /* The real case: every modifier Venus offers, decode usage, DMA_BUF.
       * This is what ffmpeg's try_export_flags() runs, and its answer decides
       * whether the frame memory is allocated exportable.
       */
      printf("\nC modifier + decode usage + profile + DMA_BUF (the real case):\n");
      int any_exportable = 0;
      for (uint32_t i = 0; i < list.drmFormatModifierCount; i++) {
         feat = 0;
         r = query(devs[d], VK_IMAGE_TILING_DRM_FORMAT_MODIFIER_EXT,
                   mods[i].drmFormatModifier, decode_usage, 1, 1, &feat);
         printf("  mod 0x%016llx : %s",
                (unsigned long long)mods[i].drmFormatModifier, res_str(r));
         if (r == VK_SUCCESS) {
            printf("  ");
            print_extmem_features(feat);
            if (feat & VK_EXTERNAL_MEMORY_FEATURE_EXPORTABLE_BIT)
               any_exportable = 1;
         }
         printf("\n");
      }

      /* Control D: modifier tiling, DMA_BUF, but WITHOUT video usage. Separates
       * "modifier export is broken" from "modifier export with video usage is
       * broken", which are different fixes.
       */
      feat = 0;
      printf("\nD modifier + sampled only + DMA_BUF (no video usage):\n");
      for (uint32_t i = 0; i < list.drmFormatModifierCount; i++) {
         feat = 0;
         r = query(devs[d], VK_IMAGE_TILING_DRM_FORMAT_MODIFIER_EXT,
                   mods[i].drmFormatModifier, VK_IMAGE_USAGE_SAMPLED_BIT, 0, 1,
                   &feat);
         printf("  mod 0x%016llx : %s",
                (unsigned long long)mods[i].drmFormatModifier, res_str(r));
         if (r == VK_SUCCESS) { printf("  "); print_extmem_features(feat); }
         printf("\n");
      }

      printf("\nVERDICT: decode-output dma-buf export %s\n",
             any_exportable ? "AVAILABLE" : "NOT AVAILABLE");
      free(mods);
      break; /* one device is the answer; a second would only confuse it */
      }
   }

   if (!found)
      printf("no non-CPU Vulkan device found\n");
   return 0;
}
