/* Positive-control tests for the W2 scrub and rejection logic.
 *
 * The point of this harness is the CONTROL, not the assertion. A test that
 * only asserts "the guest sees no video bit" passes whether or not scrubbing
 * exists, because zero is also what an unset host bit looks like. Every
 * outbound case here first asserts the value CARRIES the bit, then scrubs,
 * then asserts it is gone. If the scrub were deleted the second assertion
 * fails; if the fixture were wrong the first one does.
 *
 * Rejection cases carry the mirror control: a video value must be rejected AND
 * an ordinary value must be accepted. A reject function that returned true
 * unconditionally would close every door and break every legitimate call, and
 * only the negative control catches that.
 */

#include <stdio.h>

#include "vkr_video_scrub.h"
#include "vkr_video_reject.h"

static int failures;
static int checks;

static void
check(bool cond, const char *what)
{
   checks++;
   if (!cond) {
      failures++;
      fprintf(stderr, "FAIL: %s\n", what);
   }
}

static void
test_queue_flags(void)
{
   /* W3 INVERTED THIS. Decode is implemented, so its queue bit must now
    * SURVIVE the scrub; encode is not, so its bit must still be removed.
    *
    * Leaving the decode bit scrubbed was not a safe default, it was an
    * incoherent one: the extension is advertised on the device while the queue
    * that would carry it reports no video capability, and an application
    * correctly refuses a video queue that decodes nothing. Measured before the
    * fix -- three extensions visible, zero QUEUE_VIDEO_DECODE_BIT_KHR.
    */
   VkQueueFamilyProperties props = {
      .queueFlags = VK_QUEUE_GRAPHICS_BIT | VK_QUEUE_VIDEO_DECODE_BIT_KHR |
                    VK_QUEUE_VIDEO_ENCODE_BIT_KHR,
   };

   check((props.queueFlags & VK_QUEUE_VIDEO_DECODE_BIT_KHR) != 0,
         "fixture carries VK_QUEUE_VIDEO_DECODE_BIT_KHR before scrub");
   check((props.queueFlags & VK_QUEUE_VIDEO_ENCODE_BIT_KHR) != 0,
         "fixture carries VK_QUEUE_VIDEO_ENCODE_BIT_KHR before scrub");

   vkr_video_scrub_queue_family_properties(&props);

   check((props.queueFlags & VK_QUEUE_VIDEO_DECODE_BIT_KHR) != 0,
         "scrub PRESERVES VK_QUEUE_VIDEO_DECODE_BIT_KHR (implemented)");
   check((props.queueFlags & VK_QUEUE_VIDEO_ENCODE_BIT_KHR) == 0,
         "scrub clears VK_QUEUE_VIDEO_ENCODE_BIT_KHR (not implemented)");
   check((props.queueFlags & VK_QUEUE_GRAPHICS_BIT) != 0,
         "scrub preserves VK_QUEUE_GRAPHICS_BIT");
}

static void
test_image_layout_rejection(void)
{
   /* W3: decode layouts are the layouts a decoder must use, so rejecting them
    * would advertise an extension whose mandatory image states are refused.
    * Encode layouts stay rejected -- nothing here implements encode.
    */
   check(!vkr_video_value_VkImageLayout(VK_IMAGE_LAYOUT_VIDEO_DECODE_DPB_KHR),
         "video decode DPB layout is ACCEPTED (implemented)");
   check(!vkr_video_value_VkImageLayout(VK_IMAGE_LAYOUT_VIDEO_DECODE_DST_KHR),
         "video decode DST layout is ACCEPTED (implemented)");
   check(!vkr_video_value_VkImageLayout(VK_IMAGE_LAYOUT_VIDEO_DECODE_SRC_KHR),
         "video decode SRC layout is ACCEPTED (implemented)");

   check(vkr_video_value_VkImageLayout(VK_IMAGE_LAYOUT_VIDEO_ENCODE_DPB_KHR),
         "video ENCODE DPB layout is rejected");
   check(vkr_video_value_VkImageLayout(VK_IMAGE_LAYOUT_VIDEO_ENCODE_DST_KHR),
         "video ENCODE DST layout is rejected");
   check(vkr_video_value_VkImageLayout(VK_IMAGE_LAYOUT_VIDEO_ENCODE_SRC_KHR),
         "video ENCODE SRC layout is rejected");

   check(!vkr_video_value_VkImageLayout(VK_IMAGE_LAYOUT_UNDEFINED),
         "VK_IMAGE_LAYOUT_UNDEFINED is accepted");
   check(!vkr_video_value_VkImageLayout(VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL),
         "colour attachment layout is accepted");
   check(!vkr_video_value_VkImageLayout(VK_IMAGE_LAYOUT_PRESENT_SRC_KHR),
         "present layout is accepted");
}

static void
test_bitmask_rejection(void)
{
   /* A bitmask carries several values at once, so the check must be a mask
    * test and not equality: the guest sets the video bit ALONGSIDE legitimate
    * ones, and an equality check would sail straight past that.
    */
   /* W3: a decode usage bit alongside an ordinary bit is what a decoder
    * actually creates its images with, so it must pass. An encode bit in the
    * same position must still be refused.
    */
   check(!vkr_video_value_VkImageUsageFlags(
            VK_IMAGE_USAGE_VIDEO_DECODE_DST_BIT_KHR | VK_IMAGE_USAGE_SAMPLED_BIT),
         "video DECODE usage bit ACCEPTED when mixed with an ordinary bit");
   check(vkr_video_value_VkImageUsageFlags(
            VK_IMAGE_USAGE_VIDEO_ENCODE_DST_BIT_KHR | VK_IMAGE_USAGE_SAMPLED_BIT),
         "video ENCODE usage bit rejected when mixed with an ordinary bit");
   check(!vkr_video_value_VkImageUsageFlags(VK_IMAGE_USAGE_SAMPLED_BIT |
                                            VK_IMAGE_USAGE_TRANSFER_DST_BIT),
         "ordinary usage combination is accepted");

   /* W3: VK_QUERY_RESULT_WITH_STATUS_BIT_KHR belongs to VK_KHR_video_queue,
    * which is supported, so it is no longer a rejected value. The predicate
    * now has an empty set and returns false for everything -- kept wired so a
    * future unsupported video value of this type is caught at the call site.
    */
   check(!vkr_video_value_VkQueryResultFlags(
            VK_QUERY_RESULT_WITH_STATUS_BIT_KHR | VK_QUERY_RESULT_64_BIT),
         "WITH_STATUS bit ACCEPTED (VK_KHR_video_queue is supported)");
   check(!vkr_video_value_VkQueryResultFlags(VK_QUERY_RESULT_64_BIT |
                                             VK_QUERY_RESULT_WAIT_BIT),
         "ordinary query result flags are accepted");
}

static void
test_pnext_presence_rejection(void)
{
   /* Door 5: a video profile chained onto an ordinary create info. */
   VkVideoProfileInfoKHR profile = {
      .sType = VK_STRUCTURE_TYPE_VIDEO_PROFILE_INFO_KHR,
      .pNext = NULL,
   };
   /* W3: VkVideoProfileListInfoKHR on vkCreateImage is exactly how a decode
    * DPB image is created. Presence stopped being a violation and became the
    * required shape.
    */
   check(!vkr_video_reject_pnext(&profile),
         "video profile chained onto an ordinary create info is ACCEPTED");

   VkImageStencilUsageCreateInfo stencil = {
      .sType = VK_STRUCTURE_TYPE_IMAGE_STENCIL_USAGE_CREATE_INFO,
      .pNext = NULL,
      .stencilUsage = VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT,
   };
   check(!vkr_video_reject_pnext(&stencil),
         "ordinary chained struct with ordinary values is accepted");

   stencil.stencilUsage |= VK_IMAGE_USAGE_VIDEO_DECODE_DPB_BIT_KHR;
   /* W3: same -- a chained struct carrying a decode usage bit is the decoder
    * doing its job.
    */
   check(!vkr_video_reject_pnext(&stencil),
         "chained struct carrying a video usage bit is ACCEPTED");

   check(!vkr_video_reject_pnext(NULL), "an empty chain is accepted");
}

static void
test_barrier_rejection(void)
{
   VkImageMemoryBarrier2 barrier = {
      .sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER_2,
      .oldLayout = VK_IMAGE_LAYOUT_UNDEFINED,
      .newLayout = VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
      .srcStageMask = VK_PIPELINE_STAGE_2_TOP_OF_PIPE_BIT,
   };
   check(!vkr_video_reject_VkImageMemoryBarrier2(&barrier),
         "an ordinary image barrier is accepted");

   barrier.newLayout = VK_IMAGE_LAYOUT_VIDEO_DECODE_DPB_KHR;
   /* W3: transitioning an image into a decode layout, and the decode pipeline
    * stage, are both required by any real decode.
    */
   check(!vkr_video_reject_VkImageMemoryBarrier2(&barrier),
         "an image barrier into a video DPB layout is ACCEPTED");

   barrier.newLayout = VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL;
   barrier.srcStageMask |= VK_PIPELINE_STAGE_2_VIDEO_DECODE_BIT_KHR;
   /* W3: transitioning an image into a decode layout, and the decode pipeline
    * stage, are both required by any real decode.
    */
   check(!vkr_video_reject_VkImageMemoryBarrier2(&barrier),
         "an image barrier with a video decode stage is ACCEPTED");
}

static void
test_encode_values(void)
{
   /* The boundary is that video is absent, not that decode is absent. These
    * cases exist because the scrub masks were hand-written and covered decode
    * while encode -- and in particular the quantization-map values -- passed
    * straight through. The masks are generated now; these controls are what
    * stops them regressing to a hand-maintained subset.
    */
   VkQueueFamilyProperties props = {
      .queueFlags = VK_QUEUE_GRAPHICS_BIT | VK_QUEUE_VIDEO_ENCODE_BIT_KHR,
   };
   check((props.queueFlags & VK_QUEUE_VIDEO_ENCODE_BIT_KHR) != 0,
         "fixture carries VK_QUEUE_VIDEO_ENCODE_BIT_KHR before scrub");
   vkr_video_scrub_queue_family_properties(&props);
   check((props.queueFlags & VK_QUEUE_VIDEO_ENCODE_BIT_KHR) == 0,
         "scrub clears VK_QUEUE_VIDEO_ENCODE_BIT_KHR");
   check((props.queueFlags & VK_QUEUE_GRAPHICS_BIT) != 0,
         "scrub preserves VK_QUEUE_GRAPHICS_BIT alongside encode");

   check(vkr_video_value_VkImageLayout(VK_IMAGE_LAYOUT_VIDEO_ENCODE_SRC_KHR),
         "encode SRC layout is rejected");
   check(vkr_video_value_VkImageLayout(VK_IMAGE_LAYOUT_VIDEO_ENCODE_QUANTIZATION_MAP_KHR),
         "encode quantization-map layout is rejected");
   check(vkr_video_is_video_layout(VK_IMAGE_LAYOUT_VIDEO_ENCODE_QUANTIZATION_MAP_KHR),
         "encode quantization-map layout is filtered from outbound layout lists");
   check(!vkr_video_is_video_layout(VK_IMAGE_LAYOUT_GENERAL),
         "an ordinary layout survives outbound filtering");

   check((VKR_VIDEO_FORMAT_FEATURE_BITS2 &
          VK_FORMAT_FEATURE_2_VIDEO_ENCODE_QUANTIZATION_DELTA_MAP_BIT_KHR) != 0,
         "outbound mask covers the encode quantization-delta-map bit");
   check((VKR_VIDEO_FORMAT_FEATURE_BITS2 &
          VK_FORMAT_FEATURE_2_VIDEO_ENCODE_EMPHASIS_MAP_BIT_KHR) != 0,
         "outbound mask covers the encode emphasis-map bit");
}

static void
test_nested_attachment_reference(void)
{
   /* The value is not in the struct the pNext walker sees; it is one
    * dereference further. A chained struct can hold an attachment reference,
    * and validating the chained struct alone let a video layout inside it ride
    * through while the walker reported clean.
    */
   VkAttachmentReference2 ref = {
      .sType = VK_STRUCTURE_TYPE_ATTACHMENT_REFERENCE_2,
      .layout = VK_IMAGE_LAYOUT_VIDEO_DECODE_DPB_KHR,
   };
   VkSubpassDescriptionDepthStencilResolve resolve = {
      .sType = VK_STRUCTURE_TYPE_SUBPASS_DESCRIPTION_DEPTH_STENCIL_RESOLVE,
      .pDepthStencilResolveAttachment = &ref,
   };
   check(!vkr_video_reject_pnext(&resolve),
         "video layout in a nested depth-stencil-resolve reference is ACCEPTED");

   VkFragmentShadingRateAttachmentInfoKHR fsr = {
      .sType = VK_STRUCTURE_TYPE_FRAGMENT_SHADING_RATE_ATTACHMENT_INFO_KHR,
      .pFragmentShadingRateAttachment = &ref,
   };
   check(!vkr_video_reject_pnext(&fsr),
         "video layout in a nested shading-rate reference is ACCEPTED");

   /* Negative controls: the same chains with ordinary layouts must survive,
    * and a NULL nested pointer must not be dereferenced. */
   ref.layout = VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL;
   check(!vkr_video_reject_pnext(&resolve),
         "ordinary layout in a nested resolve reference is accepted");
   resolve.pDepthStencilResolveAttachment = NULL;
   check(!vkr_video_reject_pnext(&resolve),
         "an absent nested reference is accepted, not dereferenced");
}

static void
test_descriptor_union_tags(void)
{
   /* Which union arm is live is the decoder's decision, and the hand-written
    * guard disagreed with it in both directions. These pin the agreement.
    */
   check(vkr_video_descriptor_carries_image(VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE),
         "sampled image is an image descriptor arm");
   check(vkr_video_descriptor_carries_image(VK_DESCRIPTOR_TYPE_BLOCK_MATCH_IMAGE_QCOM),
         "QCOM block-match image is an image descriptor arm");
   check(vkr_video_descriptor_carries_image(VK_DESCRIPTOR_TYPE_SAMPLE_WEIGHT_IMAGE_QCOM),
         "QCOM sample-weight image is an image descriptor arm");

   /* Negative controls. COMBINED_IMAGE_SAMPLER is the one the hand-written
    * list wrongly included: reading data.pImage for it would reinterpret a
    * different union member. */
   check(!vkr_video_descriptor_carries_image(VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER),
         "combined image sampler is NOT a pImage arm");
   check(!vkr_video_descriptor_carries_image(VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER),
         "uniform buffer is not an image descriptor arm");
}

static void
test_video_create_flags(void)
{
   /* VK_KHR_video_maintenance1 puts a video bit in the ORDINARY create flags
    * of images and buffers. No manifest row named those types, so nothing
    * checked them; the carrier set is seeded from vk.xml now rather than from
    * the manifest, which is what makes these checks exist at all.
    */
   check(vkr_video_value_VkImageCreateFlags(
            VK_IMAGE_CREATE_MUTABLE_FORMAT_BIT |
            VK_IMAGE_CREATE_VIDEO_PROFILE_INDEPENDENT_BIT_KHR),
         "video-profile-independent image create bit rejected beside an ordinary bit");
   check(!vkr_video_value_VkImageCreateFlags(
            VK_IMAGE_CREATE_MUTABLE_FORMAT_BIT | VK_IMAGE_CREATE_ALIAS_BIT),
         "ordinary image create flags are accepted");

   check(vkr_video_value_VkBufferCreateFlags(
            VK_BUFFER_CREATE_SPARSE_BINDING_BIT |
            VK_BUFFER_CREATE_VIDEO_PROFILE_INDEPENDENT_BIT_KHR),
         "video-profile-independent buffer create bit rejected beside an ordinary bit");
   check(!vkr_video_value_VkBufferCreateFlags(VK_BUFFER_CREATE_SPARSE_BINDING_BIT),
         "ordinary buffer create flags are accepted");
}

static void
test_query_info_flags(void)
{
   /* The create paths were guarded by a hand-written call-site set, so these
    * query structs carried the same flags unchecked. Validators now cover
    * every carrier-typed member, so no call-site wiring decides it.
    */
   VkPhysicalDeviceImageFormatInfo2 img = {
      .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_FORMAT_INFO_2,
      .flags = VK_IMAGE_CREATE_VIDEO_PROFILE_INDEPENDENT_BIT_KHR,
   };
   check(vkr_video_reject_VkPhysicalDeviceImageFormatInfo2(&img),
         "video create flag rejected on the image format query");
   img.flags = VK_IMAGE_CREATE_MUTABLE_FORMAT_BIT;
   check(!vkr_video_reject_VkPhysicalDeviceImageFormatInfo2(&img),
         "ordinary create flag accepted on the image format query");

   VkPhysicalDeviceExternalBufferInfo buf = {
      .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_BUFFER_INFO,
      .flags = VK_BUFFER_CREATE_VIDEO_PROFILE_INDEPENDENT_BIT_KHR,
   };
   check(vkr_video_reject_VkPhysicalDeviceExternalBufferInfo(&buf),
         "video create flag rejected on the external buffer query");
   buf.flags = VK_BUFFER_CREATE_SPARSE_BINDING_BIT;
   check(!vkr_video_reject_VkPhysicalDeviceExternalBufferInfo(&buf),
         "ordinary create flag accepted on the external buffer query");
}

static void
test_queue_video_codec_ops(void)
{
   /* The codec-operation filter had NO control until a mutation found it:
    * replacing the mask with ~0u changed nothing observable, because nothing
    * asserted on it. An unasserted filter is indistinguishable from an absent
    * one.
    *
    * It matters because the decode queue bit and the codec list are two
    * separate obligations. A queue advertising the decode bit with an empty or
    * over-broad codec list is a queue an application cannot use correctly:
    * empty means "decodes nothing" and it declines, over-broad means it may
    * attempt a codec the renderer cannot carry.
    */
   VkQueueFamilyVideoPropertiesKHR video = {
      .sType = VK_STRUCTURE_TYPE_QUEUE_FAMILY_VIDEO_PROPERTIES_KHR,
      .videoCodecOperations = VK_VIDEO_CODEC_OPERATION_DECODE_H264_BIT_KHR |
                              VK_VIDEO_CODEC_OPERATION_DECODE_H265_BIT_KHR |
                              VK_VIDEO_CODEC_OPERATION_ENCODE_H264_BIT_KHR,
   };
   VkQueueFamilyProperties2 props = {
      .sType = VK_STRUCTURE_TYPE_QUEUE_FAMILY_PROPERTIES_2,
      .pNext = &video,
      .queueFamilyProperties = {
         .queueFlags = VK_QUEUE_VIDEO_DECODE_BIT_KHR |
                       VK_QUEUE_VIDEO_ENCODE_BIT_KHR,
      },
   };

   check((video.videoCodecOperations &
          VK_VIDEO_CODEC_OPERATION_ENCODE_H264_BIT_KHR) != 0,
         "fixture carries an ENCODE codec op before scrub");

   vkr_video_scrub_queue_family_properties2_array(&props, 1);

   check((video.videoCodecOperations &
          VK_VIDEO_CODEC_OPERATION_DECODE_H264_BIT_KHR) != 0,
         "scrub PRESERVES H.264 decode codec op (implemented)");
   check((video.videoCodecOperations &
          VK_VIDEO_CODEC_OPERATION_ENCODE_H264_BIT_KHR) == 0,
         "scrub clears the ENCODE codec op");
   check((video.videoCodecOperations &
          VK_VIDEO_CODEC_OPERATION_DECODE_H265_BIT_KHR) == 0,
         "scrub clears H.265 decode (not carried by this renderer)");
   check(video.videoCodecOperations ==
            (VkVideoCodecOperationFlagsKHR)
               VK_VIDEO_CODEC_OPERATION_DECODE_H264_BIT_KHR,
         "codec ops reduced to exactly H.264 decode");

   check((props.queueFamilyProperties.queueFlags &
          VK_QUEUE_VIDEO_DECODE_BIT_KHR) != 0,
         "array scrub preserves the decode queue bit");
   check((props.queueFamilyProperties.queueFlags &
          VK_QUEUE_VIDEO_ENCODE_BIT_KHR) == 0,
         "array scrub clears the encode queue bit");
}

int
main(void)
{
   test_queue_flags();
   test_queue_video_codec_ops();
   test_image_layout_rejection();
   test_bitmask_rejection();
   test_pnext_presence_rejection();
   test_barrier_rejection();
   test_encode_values();
   test_nested_attachment_reference();
   test_descriptor_union_tags();
   test_video_create_flags();
   test_query_info_flags();

   printf("%d checks, %d failures\n", checks, failures);
   return failures ? 1 : 0;
}
