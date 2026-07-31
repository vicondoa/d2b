/* Query the DRM format modifiers Venus reports for the H.264 decode format.
 *
 * Why this exists: vulkaninfo in this build does not print
 * VkDrmFormatModifierPropertiesListEXT at all -- verified by checking that it
 * prints none for llvmpipe either, which definitely has them. So "grep found
 * no modifiers" was a statement about the tool, not about the driver.
 *
 * This matters because Firefox's Vulkan video decoder chooses between two
 * paths based on exactly this query:
 *
 *   drmModsAreLinearOrEmpty = (mods[0] == LINEAR) && (mods.size() == 1)
 *   if (DirectExportEnabled && !drmModsAreLinearOrEmpty) -> direct export
 *   else                                                 -> copy via GL blit
 *
 * The copy path performs a GL BlitTextureToTexture that fails on virgl,
 * wedging the context. So whether this query returns a tiled modifier decides
 * whether video presents at all.
 *
 * Prints, per modifier: the modifier, its plane count, and whether its tiling
 * features carry the video decode bits Firefox requires.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <vulkan/vulkan.h>

static const char *yn(int b) { return b ? "YES" : "no"; }

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

   for (uint32_t d = 0; d < n; d++) {
      VkPhysicalDeviceProperties props;
      vkGetPhysicalDeviceProperties(devs[d], &props);
      /* Only the Venus device backed by the real GPU is interesting; llvmpipe
       * is present too and would make the output ambiguous.
       */
      if (!strstr(props.deviceName, "Venus") ||
          !strstr(props.deviceName, "NVIDIA"))
         continue;

      printf("device: %s\n", props.deviceName);

      /* The DRM node Venus claims to be. Firefox compares THIS against the
       * renderer's node (stat of gfxVars::DrmRenderDevice) to decide whether
       * the decode device and the compositor are the same GPU. A mismatch
       * makes it treat them as separate devices.
       */
      VkPhysicalDeviceDrmPropertiesEXT drm = {
         .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DRM_PROPERTIES_EXT,
      };
      VkPhysicalDeviceProperties2 p2 = {
         .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROPERTIES_2,
         .pNext = &drm,
      };
      vkGetPhysicalDeviceProperties2(devs[d], &p2);
      printf("  vendorID=0x%x deviceID=0x%x\n", props.vendorID, props.deviceID);
      printf("  DRM hasPrimary=%d primary=%lld,%lld  hasRender=%d render=%lld,%lld\n",
             drm.hasPrimary, (long long)drm.primaryMajor,
             (long long)drm.primaryMinor, drm.hasRender,
             (long long)drm.renderMajor, (long long)drm.renderMinor);
      printf("  (guest renderD128 is 226,128 -- Firefox needs a match here)\n");

      const VkFormat fmts[] = {
         VK_FORMAT_G8_B8R8_2PLANE_420_UNORM,
      };
      for (size_t f = 0; f < sizeof(fmts) / sizeof(fmts[0]); f++) {
         /* Two-call idiom: count first, then fill. */
         VkDrmFormatModifierPropertiesListEXT list = {
            .sType = VK_STRUCTURE_TYPE_DRM_FORMAT_MODIFIER_PROPERTIES_LIST_EXT,
         };
         VkFormatProperties2 fp = {
            .sType = VK_STRUCTURE_TYPE_FORMAT_PROPERTIES_2,
            .pNext = &list,
         };
         vkGetPhysicalDeviceFormatProperties2(devs[d], fmts[f], &fp);

         printf("format G8_B8R8_2PLANE_420_UNORM: %u modifier(s)\n",
                list.drmFormatModifierCount);
         printf("  optimalTilingFeatures=0x%llx linearTilingFeatures=0x%llx\n",
                (unsigned long long)fp.formatProperties.optimalTilingFeatures,
                (unsigned long long)fp.formatProperties.linearTilingFeatures);
         printf("  DECODE_OUTPUT in optimal: %s   DECODE_DPB in optimal: %s\n",
                yn(fp.formatProperties.optimalTilingFeatures &
                   VK_FORMAT_FEATURE_VIDEO_DECODE_OUTPUT_BIT_KHR),
                yn(fp.formatProperties.optimalTilingFeatures &
                   VK_FORMAT_FEATURE_VIDEO_DECODE_DPB_BIT_KHR));

         if (!list.drmFormatModifierCount) {
            printf("  NO MODIFIERS REPORTED -- Firefox falls back to LINEAR "
                   "only, which disables direct export.\n");
            continue;
         }

         VkDrmFormatModifierPropertiesEXT *mods =
            calloc(list.drmFormatModifierCount, sizeof(*mods));
         list.pDrmFormatModifierProperties = mods;
         vkGetPhysicalDeviceFormatProperties2(devs[d], fmts[f], &fp);

         int tiled = 0;
         for (uint32_t i = 0; i < list.drmFormatModifierCount; i++) {
            const VkFormatFeatureFlags tf =
               mods[i].drmFormatModifierTilingFeatures;
            printf("  modifier 0x%016llx planes=%u features=0x%llx "
                   "DECODE_OUTPUT=%s DECODE_DPB=%s TRANSFER_SRC=%s\n",
                   (unsigned long long)mods[i].drmFormatModifier,
                   mods[i].drmFormatModifierPlaneCount,
                   (unsigned long long)tf,
                   yn(tf & VK_FORMAT_FEATURE_VIDEO_DECODE_OUTPUT_BIT_KHR),
                   yn(tf & VK_FORMAT_FEATURE_VIDEO_DECODE_DPB_BIT_KHR),
                   yn(tf & VK_FORMAT_FEATURE_TRANSFER_SRC_BIT));
            if (mods[i].drmFormatModifier != 0)
               tiled++;
         }
         printf("  tiled (non-LINEAR) modifiers: %d\n", tiled);
         printf("  VERDICT: %s\n",
                tiled ? "direct export is possible (needs the pref enabled)"
                      : "LINEAR only -- direct export disabled, copy path "
                        "forced, GL blit fails");
         free(mods);
      }
   }

   free(devs);
   vkDestroyInstance(inst, NULL);
   return 0;
}
