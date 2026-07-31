#!/usr/bin/env python3
"""Emit vkr_video_reject.h from the carrying-site manifest.

The manifest is the authority for which (struct, member) pairs carry a video
value inbound. Hand-listing that set has been wrong every single time it was
attempted in this lab -- array caps 4 vs 10, pNext entry points 2 vs 5, video
values 59 vs 103 -- so the rejection table is generated from the same file that
defines the obligation. If the manifest grows a row, this grows a check.

Scope: carriers that are ORDINARY Vulkan types. A video-typed carrier (a
VkVideo* struct or enum) reaching an ordinary command is a pNext-injection
problem, closed structurally by rejecting the struct rather than by validating
its fields, and is deliberately not emitted here.
"""

import re
import sys
import pathlib
import xml.etree.ElementTree as ET

VIDEO_TYPE = re.compile(r'^(Vk|Std)Video')


def pointer_members(vk_xml):
    """(struct, member) pairs whose member is a pointer in vk.xml.

    A pointer member is an ARRAY, and a scalar value check applied to it tests
    the pointer rather than the values behind it -- pCopySrcLayouts is a
    `const VkImageLayout*`, so `vkr_video_value_VkImageLayout(s->pCopySrcLayouts)`
    is a type error.

    Parsed as XML rather than scraped with a regex. vk.xml puts a struct's
    closing tag on its own line while members close inline, so a non-greedy
    body pattern runs past the next struct and silently swallows it; the same
    quirk cost the site manifest 214 rows, and two attempts at anchoring
    around it here still missed VkPhysicalDeviceHostImageCopyProperties. It is
    an XML document, so it gets an XML parser.
    """
    root = ET.parse(vk_xml).getroot()
    ptrs = set()
    for t in root.iter('type'):
        if t.get('category') != 'struct':
            continue
        struct = t.get('name')
        if not struct:
            continue
        for mem in t.findall('member'):
            name = mem.find('name')
            typ = mem.find('type')
            if name is None or name.text is None or typ is None:
                continue
            # The `*` lives in the tail text between <type> and <name>.
            if typ.tail and '*' in typ.tail:
                ptrs.add((struct, name.text))
    return ptrs


def ordinary_reachable(row):
    """Inbound rows a guest can reach through a non-video command."""
    if row['direction'] == 'outbound':
        return False
    paths = [p for p in row['paths'] if p]
    if not paths:
        return False
    return any('Video' not in p for p in paths)


def load(path):
    rows = []
    for line in pathlib.Path(path).read_text().splitlines():
        if not line.strip():
            continue
        cols = line.split('\t')
        if len(cols) < 6:
            continue
        rows.append({
            'struct': cols[0],
            'member': cols[1],
            'carrier': cols[2],
            'direction': cols[3],
            'values': [v for v in cols[4].split(',') if v],
            'paths': cols[5].split(','),
        })
    return rows


def struct_stypes(vk_xml):
    """struct -> its VkStructureType value, for structs that chain via pNext.

    A pNext-carried struct is never reached by validating the struct that
    chains it: the guest attaches it to an ordinary create info and the values
    ride in from the side. Walking the chain needs the sType tag, which is
    recorded in vk.xml on the sType member itself.
    """
    root = ET.parse(vk_xml).getroot()
    stypes = {}
    for t in root.iter('type'):
        if t.get('category') != 'struct' or not t.get('structextends'):
            continue
        name = t.get('name')
        for mem in t.findall('member'):
            nm = mem.find('name')
            if nm is not None and nm.text == 'sType' and mem.get('values'):
                stypes[name] = mem.get('values')
    return stypes


# The single hand-written input to the whole rejection surface.
#
# Everything else is derived. These three extension names are the ONLY place a
# human states what the renderer supports; every value, struct, pNext arm and
# scrub mask follows from intersecting vk.xml against this set. A value
# contributed exclusively by an extension NOT named here lands in the reject
# path automatically, including values added by a future vk.xml revision.
#
# That direction matters. W2's defect, found twelve times across 23 panel
# rounds, was always a hand-written set deciding whether a guard applies. Going
# the other way -- hand-listing what to reject -- would reproduce it exactly,
# because the list would be complete only until the registry changed. Naming
# what is supported makes the default deny.
#
# Adding a name here is a claim that the renderer implements and wires EVERY
# command of that extension. It is not a claim that the protocol can serialize
# it.
SUPPORTED_VIDEO_EXTENSIONS = frozenset({
    'VK_KHR_video_queue',
    'VK_KHR_video_decode_queue',
    'VK_KHR_video_decode_h264',
})


def video_extension_values(vk_xml):
    """type -> set of enum values added by an UNSUPPORTED VK_*video* extension.

    Derived from the extension a value is declared in, not from its spelling.
    The manifest's value sets came from walking the three DECODE extensions
    plus dependency edges, so encode values arrived only where a dependency
    happened to drag them in: VkBufferUsageFlags2 got the encode bits while
    VkImageLayout and VkQueueFlags did not. That left
    VK_IMAGE_LAYOUT_VIDEO_ENCODE_SRC_KHR and VK_QUEUE_VIDEO_ENCODE_BIT_KHR
    riding ordinary carriers untouched.

    Values contributed by a SUPPORTED extension are excluded, because the
    renderer can now execute them -- rejecting them would advertise an
    extension whose values are refused on arrival, which is a worse failure
    than not advertising it: the guest sees support, uses it, and gets an
    error the spec does not permit at that call.

    A value declared by both a supported and an unsupported extension stays
    REJECTED. Supported-ness has to hold for EVERY extension that contributes
    a value, or an encode extension could launder a value into the allowed set
    merely by also being declared in a decode one. That is why supported
    extensions are skipped rather than subtracted: subtracting would remove a
    value from the reject set on the strength of one declaration while another,
    unsupported, extension still reaches it.
    """
    root = ET.parse(vk_xml).getroot()
    by_type = {}
    for ext in root.iter('extension'):
        name = ext.get('name') or ''
        if 'video' not in name.lower():
            continue
        if name in SUPPORTED_VIDEO_EXTENSIONS:
            continue
        for en in ext.iter('enum'):
            extends, value = en.get('extends'), en.get('name')
            if extends and value:
                by_type.setdefault(extends, set()).add(value)
    return by_type


def supported_extension_structs(vk_xml):
    """Struct type names owned by a SUPPORTED VK_*video* extension.

    Presence of one of these on an ordinary command is the REQUIRED shape once
    the extension is supported, not a violation. VkVideoProfileListInfoKHR
    chained onto VkImageCreateInfo is how every decode DPB image is created;
    rejecting it advertises an extension whose mandatory allocation is refused.
    """
    root = ET.parse(vk_xml).getroot()
    owned = set()
    for ext in root.iter('extension'):
        if (ext.get('name') or '') not in SUPPORTED_VIDEO_EXTENSIONS:
            continue
        for ty in ext.iter('type'):
            name = ty.get('name')
            if name:
                owned.add(name)
    return owned


def supported_extension_values(vk_xml):
    """type -> set of enum values added by a SUPPORTED VK_*video* extension.

    The mirror of video_extension_values(). Used to subtract decode values from
    manifest rows written while the boundary was "video is entirely absent", so
    those rows keep naming the right carrying SITES without continuing to
    reject values the renderer can now execute.

    Also propagates support across a FlagBits -> FlagBits2 widening. The 64-bit
    variants are declared by VK_KHR_maintenance5 rather than by any video
    extension, so VK_BUFFER_USAGE_2_VIDEO_DECODE_SRC_BIT_KHR is not reachable
    from the supported-extension scan even though
    VK_BUFFER_USAGE_VIDEO_DECODE_SRC_BIT_KHR is, and the decode bit would keep
    being rejected in its widened form only.

    The propagation is by BIT POSITION, not by name. A bit means the same thing
    at the same position in both types -- that is what makes the widening a
    widening -- whereas matching on spelling is the heuristic this generator
    exists to avoid, and would quietly pair unrelated bits that happen to share
    a substring.
    """
    root = ET.parse(vk_xml).getroot()
    by_type = {}
    for ext in root.iter('extension'):
        name = ext.get('name') or ''
        if name not in SUPPORTED_VIDEO_EXTENSIONS:
            continue
        for en in ext.iter('enum'):
            extends, value = en.get('extends'), en.get('name')
            if extends and value:
                by_type.setdefault(extends, set()).add(value)

    # bitpos -> value name, per type, across the whole registry.
    positions = {}
    for en in root.iter('enum'):
        extends, value, bitpos = (en.get('extends'), en.get('name'),
                                  en.get('bitpos'))
        if extends and value and bitpos is not None:
            positions.setdefault(extends, {})[bitpos] = value
    for enums in root.iter('enums'):
        tname = enums.get('name')
        if not tname:
            continue
        for en in enums.iter('enum'):
            value, bitpos = en.get('name'), en.get('bitpos')
            if value and bitpos is not None:
                positions.setdefault(tname, {})[bitpos] = value

    for base, values in list(by_type.items()):
        wide = base + '2'
        if wide not in positions:
            continue
        base_pos = positions.get(base, {})
        supported_pos = {p for p, v in base_pos.items() if v in values}
        widened = {positions[wide][p] for p in supported_pos
                   if p in positions[wide]}
        if widened:
            by_type.setdefault(wide, set()).update(widened)

    return by_type



def descriptor_image_tags(decoder_header):
    """VkDescriptorType values whose union arm decodes a VkImageDescriptorInfoEXT.

    Read out of the generated decoder rather than listed. The hand-written list
    was wrong in both directions: it named COMBINED_IMAGE_SAMPLER, which is not
    a pImage arm, and omitted BLOCK_MATCH_IMAGE_QCOM and
    SAMPLE_WEIGHT_IMAGE_QCOM, which are. A union tag set is exactly the kind of
    thing that looks obvious and is not.
    """
    text = pathlib.Path(decoder_header).read_text()
    tags, current = [], None
    for line in text.splitlines():
        m = re.match(r'\s*case (VK_DESCRIPTOR_TYPE_\w+):', line)
        if m:
            current = m.group(1)
        elif current and 'val->pImage = vn_cs_decoder_alloc_temp' in line:
            if current not in tags:
                tags.append(current)
            current = None
    return tags


def struct_members(vk_xml):
    """struct -> [(member, type)] for scalar (non-pointer) members."""
    root = ET.parse(vk_xml).getroot()
    out = {}
    for ty in root.iter('type'):
        if ty.get('category') != 'struct':
            continue
        rows = []
        for mem in ty.findall('member'):
            nm, tp = mem.find('name'), mem.find('type')
            if nm is None or tp is None or nm.text is None:
                continue
            if tp.tail and '*' in tp.tail:
                continue
            rows.append((nm.text, tp.text))
        out[ty.get('name')] = rows
    return out


def output_pnext_structs(device_header, chain):
    """Structs the reply encoder serialises on a given output pNext chain.

    Derived from the encoder's own switch. A rejection that returns early never
    writes these payloads, and the encoder sends them regardless, so each has to
    be zeroed -- and the set of them is exactly what the encoder enumerates, not
    something to list by hand.
    """
    text = pathlib.Path(device_header).read_text()
    m = re.search(r'vn_encode_%s_pnext\(struct vn_cs_encoder \*enc,[^)]*\)\s*\{(.*?)\n\}'
                  % re.escape(chain), text, re.S)
    if not m:
        return []
    out, body = [], m.group(1)
    for case in re.finditer(r'case (VK_STRUCTURE_TYPE_\w+):\s*\n(.*?)(?=\n\s*case |\n\s*default)',
                            body, re.S):
        stype, arm = case.group(1), case.group(2)
        cast = re.search(r'\(const (Vk\w+) \*\)pnext', arm)
        if cast:
            out.append((stype, cast.group(1)))
    return out


def main():
    rows = [r for r in load(sys.argv[1]) if ordinary_reachable(r)]
    ptrs = pointer_members(sys.argv[2])
    stypes = struct_stypes(sys.argv[2])

    # Command rows name a COMMAND and a parameter, not a struct and a member.
    # `vkCmdBlitImage` is not a type, so emitting a validator taking a
    # `const vkCmdBlitImage *` is a compile error. Command parameters arrive as
    # dispatch args rather than inside a decoded struct and need wiring at the
    # call site, so they are reported rather than emitted.
    commands = [r for r in rows if not r['struct'][:1].isupper()]
    rows = [r for r in rows if r['struct'][:1].isupper()]

    # A video-typed carrier is not a value to filter out of an ordinary field;
    # it is a structure that should not be present at all.
    rows = [r for r in rows if not VIDEO_TYPE.match(r['carrier'])]

    # Array members need an element-wise walk and a count member to bound it.
    # Emitting a scalar check for them would compile into a test of the
    # pointer, which is worse than emitting nothing because it looks handled.
    deferred = [r for r in rows if (r['struct'], r['member']) in ptrs]
    rows = [r for r in rows if (r['struct'], r['member']) not in ptrs]

    # `scalar` and `stdvideo` rows carry no enumerable value set -- there is no
    # bit pattern that makes a uint32_t "video", so there is nothing to reject.
    rows = [r for r in rows
            if r['values'] and r['values'][0] not in ('scalar', 'stdvideo',
                                                      'nested-struct', 'handle',
                                                      'reserved-zero')]

    carriers = {}
    # Carrier checks come from struct AND command rows. A carrier used only by
    # command parameters -- VkQueryResultFlags is one, reached solely through
    # vkGetQueryPoolResults and vkCmdCopyQueryPoolResults -- otherwise never
    # gets a check emitted, and the call-site wiring fails to compile.
    for r in rows + [c for c in commands
                     if not VIDEO_TYPE.match(c['carrier']) and c['values']
                     and c['values'][0] not in ('scalar', 'stdvideo',
                                                'nested-struct', 'handle',
                                                'reserved-zero')]:
        carriers.setdefault(r['carrier'], set()).update(r['values'])

    # Widen each carrier to every video value of that type, decode and encode
    # alike. The manifest defines WHICH SITES carry a value; it does not get to
    # define which values count as video, and its decode-scoped derivation left
    # encode bits passing through ordinary carriers untouched.
    # Outbound-only carriers get scrub masks instead of reject predicates.
    SCRUB_ONLY = {'VkQueueFlags', 'VkFormatFeatureFlags',
                  'VkFormatFeatureFlags2'}
    ext_values = video_extension_values(sys.argv[2])
    supported_values = supported_extension_values(sys.argv[2])
    for carrier in list(carriers):
        extra = ext_values.get(carrier, set())
        if not extra:
            # A bitmask's values are declared against its FlagBits type.
            extra = ext_values.get(carrier.replace('Flags', 'FlagBits'), set())
        carriers[carrier].update(extra)

    # Drop values the renderer now supports.
    #
    # The manifest rows were derived while the boundary was "video is entirely
    # absent", so they name decode values as things to reject. Those rows are
    # still the right INVENTORY of carrying sites -- that part is unchanged --
    # but they are no longer the right verdict for a supported value.
    #
    # Subtraction happens here, against the manifest-seeded sets, rather than
    # inside video_extension_values(): there the sets are keyed by declaring
    # extension and skipping is fail-closed, whereas here the value already
    # arrived from a manifest row that predates support.
    for carrier in list(carriers):
        allowed = supported_values.get(carrier, set())
        if not allowed:
            allowed = supported_values.get(
                carrier.replace('Flags', 'FlagBits'), set())
        if not allowed:
            continue
        # Never allow a value some unsupported video extension also reaches.
        still_rejected = ext_values.get(carrier, set()) | ext_values.get(
            carrier.replace('Flags', 'FlagBits'), set())
        carriers[carrier] -= (allowed - still_rejected)
        # Deliberately NOT deleted when empty. Call sites reference these
        # predicates by name, so dropping one breaks the build -- and silently
        # unwiring a guard is worse than an always-false one, because the call
        # site is where a future unsupported value would need to be caught. An
        # empty set emits a predicate that rejects nothing today and starts
        # rejecting automatically if vk.xml adds an unsupported video value to
        # that type.

    # Seed carriers from vk.xml, not from the manifest. The manifest says which
    # SITES carry a value; using it to decide which TYPES exist meant a type
    # video extends but the manifest never listed got no validator at all.
    # VK_KHR_video_maintenance1 adds a bit to VkImageCreateFlags and
    # VkBufferCreateFlags, and neither type appeared in any manifest row, so
    # neither was checked anywhere.
    SKIP = {'VkStructureType', 'VkResult', 'VkQueryResultStatusKHR'}
    for bits_type, values in ext_values.items():
        if VIDEO_TYPE.match(bits_type) or bits_type in SKIP:
            continue
        carrier = bits_type.replace('FlagBits', 'Flags')
        if carrier in SCRUB_ONLY:
            continue
        carriers.setdefault(carrier, set()).update(values)

    out = []
    w = out.append
    w('/* GENERATED by tests/gen-video-reject.py -- do not edit.')
    w(' *')
    w(' * Rejection of video enum and flag values arriving in ordinary fields.')
    w(' * Regenerate after changing the carrying-site manifest; the enforcement')
    w(' * gate compares this file against that manifest.')
    w(' */')
    w('')
    w('#ifndef VKR_VIDEO_REJECT_H')
    w('#define VKR_VIDEO_REJECT_H')
    w('')
    w('#include <stdbool.h>')
    w('')
    w('#include "vkr_common.h"')
    w('')

    for carrier in sorted(carriers):
        values = sorted(carriers[carrier])
        is_mask = 'Flags' in carrier
        w('static inline bool')
        w('vkr_video_value_%s(%s value)' % (carrier, carrier))
        w('{')
        if not values:
            # Every video value of this type belongs to a supported extension.
            # The predicate stays so its call sites stay wired: this is the
            # point where an unsupported value added by a later vk.xml would
            # have to be caught, and an unwired call site catches nothing.
            w('   /* No unsupported video value of this type exists today. */')
            w('   (void)value;')
            w('   return false;')
        elif is_mask:
            # A bitmask carries several values at once, so equality is wrong:
            # the guest sets the video bit alongside legitimate ones.
            w('   const %s video =' % carrier)
            for i, v in enumerate(values):
                tail = ';' if i == len(values) - 1 else ' |'
                w('      (%s)%s%s' % (carrier, v, tail))
            w('   return (value & video) != 0;')
        else:
            w('   switch (value) {')
            for v in values:
                w('   case %s:' % v)
            w('      return true;')
            w('   default:')
            w('      return false;')
            w('   }')
        w('}')
        w('')

    by_struct = {}
    for r in rows:
        by_struct.setdefault(r['struct'], []).append(r)

    # A validator checks EVERY member whose type is a carrier, not only the
    # members the manifest happened to list. Guarding the manifest's members and
    # then hand-wiring the rest at call sites is how VkImageCreateInfo::flags
    # ended up checked on the create path and unchecked on the format-query
    # path: the manifest says which sites carry a value, but the type graph is
    # what says which members can.
    all_members = struct_members(sys.argv[2])
    for struct in sorted(by_struct):
        listed = {r['member']: r['carrier'] for r in by_struct[struct]}
        for name, typ in all_members.get(struct, []):
            if typ in carriers and name not in listed:
                listed[name] = typ
        w('static inline bool')
        w('vkr_video_reject_%s(const %s *s)' % (struct, struct))
        w('{')
        w('   if (!s)')
        w('      return false;')
        for name in sorted(listed):
            w('   if (vkr_video_value_%s(s->%s))' % (listed[name], name))
            w('      return true;')
        w('   return false;')
        w('}')
        w('')

    chainable = {s: stypes[s] for s in by_struct if s in stypes}

    # Video-typed structs that chain onto an ordinary create info are door 5:
    # the guest attaches a video profile to vkCreateImage or vkCreateBuffer and
    # the values ride in from the side. For these, PRESENCE is the violation --
    # there is no legitimate value to filter, because the whole struct has no
    # business on an ordinary command while video is off.
    #
    # Once an extension IS supported, presence stops being a violation and
    # becomes the required shape: VkVideoProfileListInfoKHR on vkCreateImage is
    # exactly how a decode DPB image is created, and rejecting it means the
    # extension is advertised while the one image every decoder must allocate
    # cannot be. So a struct owned by a supported extension is excluded here.
    supported_structs = supported_extension_structs(sys.argv[2])
    video_chained = {}
    for r in load(sys.argv[1]):
        if not ordinary_reachable(r):
            continue
        for name in (r['struct'], r['carrier']):
            if (VIDEO_TYPE.match(name) and name in stypes
                    and name not in supported_structs):
                video_chained[name] = stypes[name]

    # Structs reached through an ARRAY inside a chained struct. The walker sees
    # the parent but never the elements, so these need an explicit element walk
    # keyed by the parent's sType. Kept as a small declared table rather than
    # derived, because vk.xml's `len` attribute names the count member but not
    # which chained parent is the one that matters; the enforcement gate is
    # what catches a missing entry.
    NESTED_ARRAYS = {
        'VkFramebufferAttachmentImageInfo': (
            'VkFramebufferAttachmentsCreateInfo',
            'VK_STRUCTURE_TYPE_FRAMEBUFFER_ATTACHMENTS_CREATE_INFO',
            'pAttachmentImageInfos', 'attachmentImageInfoCount'),
    }

    # Chained structs holding a NESTED VkAttachmentReference2 pointer. The
    # pNext walker dispatches by sType and validates the chained struct itself;
    # it does not dereference pointers hanging off it, so a video layout inside
    # one of these rode through while the walker reported clean.
    NESTED_REFS = {
        'VK_STRUCTURE_TYPE_SUBPASS_DESCRIPTION_DEPTH_STENCIL_RESOLVE': (
            'VkSubpassDescriptionDepthStencilResolve',
            'pDepthStencilResolveAttachment'),
        'VK_STRUCTURE_TYPE_FRAGMENT_SHADING_RATE_ATTACHMENT_INFO_KHR': (
            'VkFragmentShadingRateAttachmentInfoKHR',
            'pFragmentShadingRateAttachment'),
    }

    if chainable or video_chained:
        for struct in sorted(video_chained):
            # A named helper rather than a bare `return true` in the switch:
            # the struct name has to appear in real code for the enforcement
            # gate to see that this site is handled, and an sType constant
            # names neither the struct nor its members.
            w('static inline bool')
            w('vkr_video_reject_present_%s(const %s *s)' % (struct, struct))
            w('{')
            w('   /* Presence is the violation; no member is legitimate here. */')
            w('   return s != NULL;')
            w('}')
            w('')
        w('/* Walk a pNext chain and reject any chained struct that carries a')
        w(' * video value. A chained struct is never reached by validating the')
        w(' * struct it hangs off: the guest attaches it to an ordinary create')
        w(' * info and the values ride in from the side.')
        w(' */')
        w('static inline bool vkr_video_reject_pnext(const void *pnext);')
        w('')
        w('static inline bool')
        w('vkr_video_reject_pnext(const void *pnext)')
        w('{')
        w('   for (const VkBaseInStructure *s = pnext; s; s = s->pNext) {')
        w('      switch (s->sType) {')
        for stype in sorted(NESTED_REFS):
            struct, member = NESTED_REFS[stype]
            w('      /* %s holds a nested attachment reference. */' % struct)
            w('      case %s: {' % stype)
            w('         const %s *n = (const %s *)s;' % (struct, struct))
            w('         if (n->%s &&' % member)
            w('             (vkr_video_reject_VkAttachmentReference2(n->%s) ||' % member)
            w('              vkr_video_reject_pnext(n->%s->pNext)))' % member)
            w('            return true;')
            w('         break;')
            w('      }')
        for struct in sorted(video_chained):
            w('      /* %s: presence alone is the violation. */' % struct)
            w('      case %s:' % video_chained[struct])
            w('         return vkr_video_reject_present_%s((const %s *)s);'
              % (struct, struct))
        for struct in sorted(chainable):
            w('      case %s:' % chainable[struct])
            w('         if (vkr_video_reject_%s((const %s *)s))' % (struct, struct))
            w('            return true;')
            w('         break;')
        for struct in sorted(NESTED_ARRAYS):
            if struct not in by_struct:
                continue
            parent, stype, arr, count = NESTED_ARRAYS[struct]
            w('      /* %s elements live in an array on %s. */' % (struct, parent))
            w('      case %s: {' % stype)
            w('         const %s *p = (const %s *)s;' % (parent, parent))
            w('         for (uint32_t i = 0; i < p->%s; i++) {' % count)
            w('            if (vkr_video_reject_%s(&p->%s[i]))' % (struct, arr))
            w('               return true;')
            w('         }')
            w('         break;')
            w('      }')
        w('      default:')
        w('         break;')
        w('      }')
        w('   }')
        w('   return false;')
        w('}')
        w('')

    # Masks for the OUTBOUND scrub, derived from the same vk.xml extension data
    # as the rejection checks. These were hand-written, and hand-written is how
    # the encode quantization-map bits went missing while decode was covered:
    # a mask maintained by hand drifts from the registry the moment the
    # registry moves. The scrub header consumes these instead of defining them.
    SCRUB_TYPES = {
        'VkQueueFlagBits': 'VKR_VIDEO_QUEUE_BITS',
        'VkFormatFeatureFlagBits': 'VKR_VIDEO_FORMAT_FEATURE_BITS',
        'VkFormatFeatureFlagBits2': 'VKR_VIDEO_FORMAT_FEATURE_BITS2',
    }
    w('/* Outbound scrub masks, derived from the video extensions in vk.xml. */')
    for bits_type in sorted(SCRUB_TYPES):
        values = sorted(ext_values.get(bits_type, set()))
        macro = SCRUB_TYPES[bits_type]
        if not values:
            w('#define %s 0' % macro)
            w('')
            continue
        w('#define %s \\' % macro)
        for i, v in enumerate(values):
            w('   %s%s' % (('(' if i == 0 else ' '), v) +
              (' | \\' if i < len(values) - 1 else ')'))
        w('')

    layouts = sorted(ext_values.get('VkImageLayout', set()))
    w('/* Every video image layout, for outbound layout-list compaction. */')
    w('static inline bool')
    w('vkr_video_is_video_layout(VkImageLayout layout)')
    w('{')
    w('   switch (layout) {')
    for v in layouts:
        w('   case %s:' % v)
    w('      return true;')
    w('   default:')
    w('      return false;')
    w('   }')
    w('}')
    w('')

    if len(sys.argv) > 3:
        tags = descriptor_image_tags(sys.argv[3])
        w('/* Descriptor tags whose union arm decodes an image descriptor. */')
        w('static inline bool')
        w('vkr_video_descriptor_carries_image(VkDescriptorType type)')
        w('{')
        w('   switch (type) {')
        for tag in tags:
            w('   case %s:' % tag)
        w('      return true;')
        w('   default:')
        w('      return false;')
        w('   }')
        w('}')
        w('')

    for chain, fn in (('VkImageFormatProperties2', 'image_format'),
                      ('VkExternalBufferProperties', 'external_buffer')):
        chained = output_pnext_structs(sys.argv[4], chain) if len(sys.argv) > 4 else []
        if chained:
            w('/* Zero output pNext payloads on a reject path, preserving the')
            w(' * sType/pNext header the encoder asserts on. The set is the one')
            w(' * the reply encoder itself enumerates. */')
            w('static inline void')
            w('vkr_video_zero_%s_pnext(void *pnext)' % fn)
            w('{')
            w('   for (VkBaseOutStructure *s = pnext; s; s = s->pNext) {')
            w('      const VkStructureType st = s->sType;')
            w('      VkBaseOutStructure *next = s->pNext;')
            w('      switch (st) {')
            for stype, struct in chained:
                w('      case %s:' % stype)
                w('         memset(s, 0, sizeof(%s));' % struct)
                w('         break;')
            w('      default:')
            w('         break;')
            w('      }')
            w('      s->sType = st;')
            w('      s->pNext = next;')
            w('   }')
            w('}')
            w('')

    w('#endif /* VKR_VIDEO_REJECT_H */')
    sys.stdout.write('\n'.join(out) + '\n')
    print('gen-video-reject: %d sites, %d carriers, %d structs'
          % (len(rows), len(carriers), len(by_struct)), file=sys.stderr)
    if deferred:
        print('gen-video-reject: %d array members deferred (need element walk):'
              % len(deferred), file=sys.stderr)
        for r in sorted(deferred, key=lambda r: (r['struct'], r['member'])):
            print('   %s.%s (%s)' % (r['struct'], r['member'], r['carrier']),
                  file=sys.stderr)
    if commands:
        print('gen-video-reject: %d command parameters need call-site wiring:'
              % len(commands), file=sys.stderr)
        for r in sorted(commands, key=lambda r: (r['struct'], r['member'])):
            print('   %s(%s) (%s)' % (r['struct'], r['member'], r['carrier']),
                  file=sys.stderr)


if __name__ == '__main__':
    main()
