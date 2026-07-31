/* Negative controls for the W3 video validators.
 *
 * The asymmetry this file exists for: a no-op REJECTER is caught by a positive
 * control, because the thing it should have let through stops working. A no-op
 * VALIDATOR -- one that accepts everything -- is caught only by a negative
 * control, because everything keeps working and nothing looks wrong.
 *
 * W2 turned ~95 sites into rejections; W3 turns them into validations. That is
 * the same failure mode with the polarity reversed: a wrong rejection fails
 * closed and shows up as a feature that does not work, a wrong validation fails
 * open and shows up as nothing at all.
 *
 * So every check below is paired: at least one input it MUST reject, and at
 * least one ordinary input it MUST accept. A control that only ever asserts
 * rejection is satisfied by a validator that rejects everything, which would
 * pass this file while breaking every real decode.
 *
 * Mutation discipline: commenting out the corresponding check in
 * vkr_video_validate.h must make a control here fail. A control never observed
 * failing has not been shown to test anything.
 */

#include <inttypes.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "vkr_video_validate.h"

static unsigned checks;
static unsigned failures;

static void
check(bool ok, const char *what)
{
   checks++;
   if (!ok) {
      failures++;
      printf("  FAIL %s\n", what);
   }
}

/* --- bitstream range bounds --------------------------------------------- */

static void
test_range_bounds(void)
{
   printf("range bounds\n");

   /* Positive controls: ordinary ranges must be accepted, or a validator that
    * rejects everything would pass the negatives below.
    */
   check(vkr_video_range_within(0, 1024, 4096), "whole buffer from 0 accepted");
   check(vkr_video_range_within(1024, 1024, 4096), "mid-buffer range accepted");
   check(vkr_video_range_within(4096, 0, 4096), "empty range at end accepted");

   /* VK_WHOLE_SIZE resolves to the remainder, so it is legal at any valid
    * offset. This is the case the naive check gets wrong in BOTH directions.
    */
   check(vkr_video_range_within(0, VK_WHOLE_SIZE, 4096),
         "VK_WHOLE_SIZE at 0 accepted");
   check(vkr_video_range_within(4095, VK_WHOLE_SIZE, 4096),
         "VK_WHOLE_SIZE at last byte accepted");

   /* Negative controls. */
   check(!vkr_video_range_within(0, 4097, 4096), "range past end rejected");
   check(!vkr_video_range_within(4097, 0, 4096), "offset past end rejected");
   check(!vkr_video_range_within(2048, 2049, 4096),
         "offset+range past end rejected");

   /* THE overflow control.
    *
    * A generic too-large range still passes if the overflow-safe formulation
    * regresses to `offset + range <= size`, because that comparison is only
    * wrong when it wraps. Only an input designed to wrap detects the
    * regression: offset + range must exceed UINT64_MAX.
    */
   check(!vkr_video_range_within(UINT64_MAX - 16, 4096, 4096),
         "64-bit wrapping offset+range rejected");
   check(!vkr_video_range_within(1024, UINT64_MAX - 512, 4096),
         "64-bit wrapping range rejected");
}

/* --- reference slot ignore semantics ------------------------------------ */

static void
test_slot_ignore(void)
{
   printf("slot ignore semantics\n");

   /* A negative index means the picture is NOT retained in the DPB, which is
    * ordinary for non-reference pictures in High-profile B-frames. Treating it
    * as a real slot rejects valid content.
    */
   check(vkr_video_slot_index_is_ignored(-1), "-1 is ignored");
   check(vkr_video_slot_index_is_ignored(-42), "any negative is ignored");

   check(!vkr_video_slot_index_is_ignored(0), "slot 0 is a real slot");
   check(!vkr_video_slot_index_is_ignored(15), "slot 15 is a real slot");
}

/* --- reference slot validation ------------------------------------------ */

static struct vkr_video_session
make_session(uint32_t max_dpb_slots)
{
   struct vkr_video_session sess;
   memset(&sess, 0, sizeof(sess));
   sess.max_dpb_slots = max_dpb_slots;
   sess.max_active_references = max_dpb_slots;
   return sess;
}

static void
test_reference_slots(void)
{
   printf("reference slots\n");

   struct vkr_video_session sess = make_session(16);

   VkVideoPictureResourceInfoKHR bound;
   memset(&bound, 0, sizeof(bound));
   bound.imageViewBinding = (VkImageView)(uintptr_t)0xd2b0;

   VkVideoPictureResourceInfoKHR unbound;
   memset(&unbound, 0, sizeof(unbound));
   unbound.imageViewBinding = VK_NULL_HANDLE;

   VkVideoReferenceSlotInfoKHR slot;
   memset(&slot, 0, sizeof(slot));

   /* Positive: an in-range slot naming a bound image is ordinary. */
   slot.slotIndex = 0;
   slot.pPictureResource = &bound;
   check(vkr_video_validate_reference_slot(&sess, &slot, false),
         "in-range reference slot with bound image accepted");
   check(vkr_video_validate_reference_slot(&sess, &slot, true),
         "in-range setup slot with bound image accepted");

   slot.slotIndex = 15;
   check(vkr_video_validate_reference_slot(&sess, &slot, false),
         "last valid slot accepted");

   /* Negative: out of range. maxDpbSlots is 16, so 16 is one past the end --
    * the off-by-one a `<=` would let through.
    */
   slot.slotIndex = 16;
   check(!vkr_video_validate_reference_slot(&sess, &slot, false),
         "slot == maxDpbSlots rejected");
   slot.slotIndex = 9999;
   check(!vkr_video_validate_reference_slot(&sess, &slot, false),
         "far out-of-range slot rejected");

   /* Negative: a real slot naming an unbound image. */
   slot.slotIndex = 3;
   slot.pPictureResource = &unbound;
   check(!vkr_video_validate_reference_slot(&sess, &slot, false),
         "reference slot with null image rejected");

   /* THE setup-slot control.
    *
    * The spike exempted pSetupReferenceSlot entirely, which let decoded output
    * land on an image the session never bound. A control that only ever puts
    * the invalid handle in pReferenceSlots[] would not notice a regression
    * that re-exempts the setup slot, so this one targets it specifically.
    */
   slot.slotIndex = 3;
   slot.pPictureResource = &unbound;
   check(!vkr_video_validate_reference_slot(&sess, &slot, true),
         "SETUP slot with null image rejected");
   slot.slotIndex = 16;
   slot.pPictureResource = &bound;
   check(!vkr_video_validate_reference_slot(&sess, &slot, true),
         "SETUP slot out of range rejected");

   /* Ignored slots are accepted without touching the resource, which may be
    * uninitialised. Passing a deliberately bogus pointer proves it is not
    * dereferenced: if the validator reads it, this crashes under ASan.
    */
   slot.slotIndex = -1;
   slot.pPictureResource = (const VkVideoPictureResourceInfoKHR *)(uintptr_t)0x1;
   check(vkr_video_validate_reference_slot(&sess, &slot, true),
         "ignored setup slot accepted without dereferencing the resource");
   check(vkr_video_validate_reference_slot(&sess, &slot, false),
         "ignored reference slot accepted without dereferencing the resource");

   /* NULL setup slot is legal; NULL reference slot entries are not reachable
    * because the array is walked by count.
    */
   check(vkr_video_validate_reference_slot(&sess, NULL, true),
         "NULL setup slot accepted");
}

/* --- decode info -------------------------------------------------------- */

static void
test_decode_info(void)
{
   printf("decode info\n");

   struct vkr_video_session sess = make_session(8);

   VkVideoPictureResourceInfoKHR bound;
   memset(&bound, 0, sizeof(bound));
   bound.imageViewBinding = (VkImageView)(uintptr_t)0xd2b0;

   VkVideoReferenceSlotInfoKHR slots[4];
   memset(slots, 0, sizeof(slots));
   for (uint32_t i = 0; i < 4; i++) {
      slots[i].slotIndex = (int32_t)i;
      slots[i].pPictureResource = &bound;
   }

   VkVideoDecodeInfoKHR info;
   memset(&info, 0, sizeof(info));
   info.referenceSlotCount = 4;
   info.pReferenceSlots = slots;

   check(vkr_video_validate_decode_info(&sess, &info),
         "ordinary decode info accepted");

   /* THE count control.
    *
    * Bounding the indices says nothing about how many there are. A validator
    * that checks every element but not the count still walks past the end of
    * whatever the guest actually supplied.
    */
   info.referenceSlotCount = 9; /* maxDpbSlots is 8 */
   check(!vkr_video_validate_decode_info(&sess, &info),
         "referenceSlotCount past maxDpbSlots rejected");
   info.referenceSlotCount = 4;

   /* One bad element among good ones must still reject: a validator that
    * checks only the first entry passes the ordinary case above.
    */
   slots[2].slotIndex = 99;
   check(!vkr_video_validate_decode_info(&sess, &info),
         "out-of-range slot in the middle of the array rejected");
   slots[2].slotIndex = 2;

   check(!vkr_video_validate_decode_info(&sess, NULL),
         "NULL decode info rejected");
   check(!vkr_video_validate_decode_info(NULL, &info),
         "NULL session rejected");
}

/* --- format allowlist --------------------------------------------------- */

static void
test_format_allowlist(void)
{
   printf("format allowlist\n");

   /* Positive: the mandatory H.264 decode format, and the one FFmpeg picked. */
   check(vkr_video_format_is_allowed(VK_FORMAT_G8_B8R8_2PLANE_420_UNORM),
         "NV12 allowed");

   /* Negative: 10-bit and 12-bit are NOT carried. Nothing here decodes them,
    * and reporting them would have the guest allocate decode images the
    * renderer has never exercised.
    */
   check(!vkr_video_format_is_allowed(
            VK_FORMAT_G10X6_B10X6R10X6_2PLANE_420_UNORM_3PACK16),
         "P010 rejected");
   check(!vkr_video_format_is_allowed(
            VK_FORMAT_G12X4_B12X4R12X4_2PLANE_420_UNORM_3PACK16),
         "P012 rejected");
   check(!vkr_video_format_is_allowed(VK_FORMAT_R8G8B8A8_UNORM),
         "an ordinary colour format rejected");

   printf("decode usage allowlist\n");

   check(vkr_video_decode_usage_is_allowed(VK_IMAGE_USAGE_VIDEO_DECODE_DST_BIT_KHR),
         "decode DST allowed");
   check(vkr_video_decode_usage_is_allowed(VK_IMAGE_USAGE_VIDEO_DECODE_DPB_BIT_KHR),
         "decode DPB allowed");
   check(vkr_video_decode_usage_is_allowed(
            VK_IMAGE_USAGE_VIDEO_DECODE_DST_BIT_KHR | VK_IMAGE_USAGE_SAMPLED_BIT),
         "decode DST plus sampled allowed");

   /* Zero is not a usage. A validator using a plain mask test accepts it,
    * which would forward a meaningless query to the host driver.
    */
   check(!vkr_video_decode_usage_is_allowed(0), "empty usage rejected");

   /* ENCODE usage must not reach the host through the decode query. This is
    * the control that catches the allowlist being widened to "any video bit".
    */
   check(!vkr_video_decode_usage_is_allowed(VK_IMAGE_USAGE_VIDEO_ENCODE_SRC_BIT_KHR),
         "encode SRC rejected");
   check(!vkr_video_decode_usage_is_allowed(
            VK_IMAGE_USAGE_VIDEO_DECODE_DST_BIT_KHR |
            VK_IMAGE_USAGE_VIDEO_ENCODE_DPB_BIT_KHR),
         "decode bit mixed with an encode bit rejected");

   /* A mask test that only checks for a video bit being PRESENT accepts this;
    * only checking for disallowed bits being ABSENT rejects it.
    */
   check(!vkr_video_decode_usage_is_allowed(
            VK_IMAGE_USAGE_VIDEO_DECODE_DST_BIT_KHR |
            VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT),
         "decode bit mixed with a non-video bit rejected");
}

int
main(void)
{
   printf("=== W3 video validator controls ===\n");
   test_range_bounds();
   test_slot_ignore();
   test_reference_slots();
   test_decode_info();
   test_format_allowlist();
   printf("=== %u checks, %u failures ===\n", checks, failures);
   return failures ? 1 : 0;
}
