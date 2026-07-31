/* Standalone shim so the scrub, rejection and validation headers compile
 * outside the renderer. They use Vulkan types plus a small amount of the vkr
 * object model; the real vkr_common.h drags in the whole renderer, which this
 * harness has no need for and must not depend on.
 *
 * Everything here is the MINIMUM the headers under test reference. Keeping it
 * minimal is the point: a shim that grew toward the real header would start
 * hiding whether a validator depends on renderer state it should not.
 */
#ifndef VKR_COMMON_H
#define VKR_COMMON_H

#include <assert.h>
#include <stdbool.h>
#include <stdint.h>
#include <string.h>

#include <vulkan/vulkan.h>

typedef uint64_t vkr_object_id;

/* Mirrors the layout the validators touch: type, id, and the handle union.
 * The real struct carries more, none of which any validator may read -- if a
 * validator ever needs a field absent here, that is a signal it has reached
 * into renderer state rather than validating its arguments.
 */
struct vkr_context;

struct vkr_object {
   VkObjectType type;
   vkr_object_id id;

   union {
      uint64_t u64;
      VkVideoSessionKHR video_session;
      VkVideoSessionParametersKHR video_session_parameters;
   } handle;
};

#define VKR_DEFINE_OBJECT_CAST(vkr_type, vk_enum, vk_type)                               \
   static inline struct vkr_##vkr_type *vkr_##vkr_type##_from_handle(vk_type handle)     \
   {                                                                                     \
      return (struct vkr_##vkr_type *)(uintptr_t)handle;                                 \
   }

#endif /* VKR_COMMON_H */
