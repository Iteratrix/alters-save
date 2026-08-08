//! Item inventory: the `P9ItemStack` list inside the `P9ResourceSubsystem`
//! record.
//!
//! Layout (all inside the subsystem record's sized payload):
//!
//! ```text
//! [i32 list_size] [marker 2e 41 2e 41] [i32 stack_count] [i32 0]
//! [stack record]*
//! ```
//!
//! Each stack record:
//!
//! ```text
//! [lstr "/Script/P9Playable.P9ItemStack"] [lstr "P9ItemStack_<id>"]
//! [i32 0] [i32 s1] [content1:
//!     u8 0
//!     Count: IntProperty, Reserved: IntProperty, lstr "None"
//!     [i32 0] [i32 s2] [content2:
//!         marker 2e 01 2e 01, i32 0,
//!         [lstr item class path] [lstr "BP_Item_<X>_C_<id>"]
//!         [i32 0] [i32 s3] [content3: u8 0, tagged props..., lstr "None", i32 0]
//!         ...tail bytes...]
//!     ...tail bytes...]
//! ...tail bytes to next record...
//! ```
//!
//! The sized spans (`s1`/`s2`/`s3`) cut through the trailing
//! `/Engine/Transient` reference strings rather than ending on logical
//! boundaries, so this module never rebuilds records from scratch. New
//! stacks are cloned from an existing record - every byte outside the
//! swapped strings and emptied property list is preserved verbatim, and the
//! three sizes are adjusted by the length deltas. An emptied property list
//! makes the game construct the item from its class defaults, which is the
//! behavior verified in-game.
//!
//! Four size fields enclose an insertion and must all grow by the record
//! length: the stack list size, the subsystem record size, the whole-body
//! size at offset 0, and (outside this module) the two EOF-relative prefix
//! fields rewritten by [`crate::sav::SaveFile::to_bytes`].

use memchr::memmem;

use crate::error::{Error, Result};
use crate::sav::ArchiveVersion;

const STACK_SCRIPT: &[u8] = b"/Script/P9Playable.P9ItemStack\0";
const SUBSYSTEM_SCRIPT: &[u8] = b"/Script/P9Playable.P9ResourceSubsystem\0";
const LIST_MARKER: &[u8] = &[0x2e, 0x41, 0x2e, 0x41];
const COUNT_PROP_V3: &[u8] =
    b"\x06\x00\x00\x00Count\x00\x0c\x00\x00\x00IntProperty\x00\x00\x00\x00\x00\x04\x00\x00\x00\x00";
const COUNT_PROP_V2: &[u8] =
    b"\x06\x00\x00\x00Count\x00\x0c\x00\x00\x00IntProperty\x00\x04\x00\x00\x00\x00\x00\x00\x00\x00";
const EMBEDDED_MARKER: &[u8] = &[0x2e, 0x01, 0x2e, 0x01];
const EMPTY_PROPS: &[u8] = b"\x00\x05\x00\x00\x00None\x00\x00\x00\x00\x00";
const ITEM_PATH_PREFIX: &str = "/Game/P9Playable/Items/";

fn count_prop_pattern(version: ArchiveVersion) -> Result<&'static [u8]> {
    match version {
        ArchiveVersion::V3 => Ok(COUNT_PROP_V3),
        ArchiveVersion::V2 => Ok(COUNT_PROP_V2),
        ArchiveVersion::Other(value) => Err(Error::UnsupportedArchiveVersion(value)),
    }
}

/// An item class stem, e.g. `BridgePylon` or `RepairKit`
/// (`BP_Item_<stem>_C` in class paths).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ItemClass(pub String);

/// One item stack in the inventory list.
#[derive(Debug, Clone)]
pub struct Stack {
    pub item: ItemClass,
    pub count: i32,
    record_start: usize,
    record_end: usize,
    count_offset: usize,
}

fn read_i32(bytes: &[u8], offset: usize) -> Option<i32> {
    let slice = bytes.get(offset..offset.checked_add(4)?)?;
    let array: [u8; 4] = slice.try_into().ok()?;
    Some(i32::from_le_bytes(array))
}

fn read_usize(bytes: &[u8], offset: usize) -> Option<usize> {
    usize::try_from(read_i32(bytes, offset)?).ok()
}

fn read_lstr(bytes: &[u8], offset: usize) -> Option<(&[u8], usize)> {
    let len = read_usize(bytes, offset)?;
    let start = offset.checked_add(4)?;
    let end = start.checked_add(len)?;
    let content = bytes.get(start..end)?;
    let (&0, text) = content.split_last()? else {
        return None;
    };
    Some((text, end))
}

fn lstr(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 5);
    let len = i32::try_from(text.len() + 1).expect("string length fits i32");
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(text);
    out.push(0);
    out
}

fn structure_error(reason: &str) -> Error {
    Error::MalformedChunk {
        offset: 0,
        reason: format!("item list structure: {reason}"),
    }
}

/// The located inventory list plus the enclosing size fields.
#[derive(Debug)]
pub struct Inventory {
    pub stacks: Vec<Stack>,
    list_size_offset: usize,
    stack_count_offset: usize,
    subsystem_size_offset: usize,
    first_record_start: usize,
}

fn item_class_of(record: &[u8]) -> Option<ItemClass> {
    let prefix = ITEM_PATH_PREFIX.as_bytes();
    let start = memmem::find(record, prefix)? + prefix.len();
    let tail = &record[start..];
    let dot = tail.iter().position(|&b| b == b'.')?;
    let stem = std::str::from_utf8(&tail[..dot]).ok()?;
    let stem = stem.strip_prefix("BP_Item_").unwrap_or(stem);
    Some(ItemClass(stem.to_owned()))
}

/// Locate the item stack list in a decompressed body.
///
/// # Errors
///
/// Returns an error when any structural invariant fails: missing subsystem
/// record, missing or inconsistent list header, or stack records whose
/// declared spans disagree with the list span.
pub fn inventory(body: &[u8], version: ArchiveVersion) -> Result<Inventory> {
    let count_prop = count_prop_pattern(version)?;
    let Some(subsystem_script) = memmem::find(body, SUBSYSTEM_SCRIPT) else {
        return Err(structure_error("P9ResourceSubsystem record not found"));
    };
    let (_name, after_name) = read_lstr(body, subsystem_script + SUBSYSTEM_SCRIPT.len())
        .ok_or_else(|| structure_error("subsystem instance name"))?;
    if read_i32(body, after_name) != Some(0) {
        return Err(structure_error("subsystem pre-size pad"));
    }
    let subsystem_size_offset = after_name + 4;
    let subsystem_size =
        read_usize(body, subsystem_size_offset).ok_or_else(|| structure_error("subsystem size"))?;
    let subsystem_end = subsystem_size_offset + 4 + subsystem_size;

    let record_starts: Vec<usize> = memmem::find_iter(body, STACK_SCRIPT)
        .filter_map(|script| script.checked_sub(4))
        .filter(|&start| {
            read_usize(body, start) == Some(STACK_SCRIPT.len())
                && start > subsystem_script
                && start < subsystem_end
        })
        .collect();

    let Some(&first_record_start) = record_starts.first() else {
        return Err(structure_error(
            "no item stacks in save (produce any item in-game once, then retry)",
        ));
    };

    let header = body
        .get(first_record_start - 16..first_record_start)
        .ok_or_else(|| structure_error("list header out of range"))?;
    if &header[4..8] != LIST_MARKER || header[12..16] != [0, 0, 0, 0] {
        return Err(structure_error("list header marker mismatch"));
    }
    let list_size_offset = first_record_start - 16;
    let stack_count_offset = first_record_start - 8;
    let list_size =
        read_usize(body, list_size_offset).ok_or_else(|| structure_error("list size"))?;
    let declared_count =
        read_usize(body, stack_count_offset).ok_or_else(|| structure_error("stack count"))?;
    if declared_count != record_starts.len() {
        return Err(structure_error("stack count disagrees with records found"));
    }
    let list_end = list_size_offset + 4 + list_size;
    if list_end > subsystem_end {
        return Err(structure_error("list extends past subsystem record"));
    }

    let stacks = record_starts
        .iter()
        .enumerate()
        .map(|(index, &record_start)| {
            let record_end = record_starts.get(index + 1).copied().unwrap_or(list_end);
            let record = &body[record_start..record_end];
            let item = item_class_of(record)
                .ok_or_else(|| structure_error("stack without item class path"))?;
            let count_in_record = memmem::find(record, count_prop)
                .ok_or_else(|| structure_error("stack without Count property"))?;
            let count_offset = record_start + count_in_record + count_prop.len();
            let count = read_i32(body, count_offset)
                .ok_or_else(|| structure_error("truncated Count value"))?;
            Ok(Stack {
                item,
                count,
                record_start,
                record_end,
                count_offset,
            })
        })
        .collect::<Result<Vec<Stack>>>()?;

    Ok(Inventory {
        stacks,
        list_size_offset,
        stack_count_offset,
        subsystem_size_offset,
        first_record_start,
    })
}

/// Overwrite the count of an existing stack (fixed-width, in-place).
///
/// # Errors
///
/// Returns [`Error::OutOfBounds`] if the stack's offsets do not fit `body`.
pub fn set_count(body: &mut [u8], stack: &Stack, count: i32) -> Result<()> {
    let Stack { count_offset, .. } = *stack;
    let Some(slot) = body.get_mut(count_offset..count_offset + 4) else {
        return Err(Error::OutOfBounds {
            what: "stack count field",
            value: count_offset,
            len: body.len(),
        });
    };
    slot.copy_from_slice(&count.to_le_bytes());
    Ok(())
}

fn splice(buffer: &[u8], start: usize, old_len: usize, replacement: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(buffer.len() - old_len + replacement.len());
    out.extend_from_slice(&buffer[..start]);
    out.extend_from_slice(replacement);
    out.extend_from_slice(&buffer[start + old_len..]);
    out
}

fn write_i32(buffer: &mut [u8], offset: usize, value: i32) {
    buffer[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn adjust_i32(buffer: &mut [u8], offset: usize, delta: i64) -> Result<()> {
    let current = read_i32(buffer, offset).ok_or(Error::OutOfBounds {
        what: "size field",
        value: offset,
        len: buffer.len(),
    })?;
    let adjusted = i64::from(current) + delta;
    let adjusted = i32::try_from(adjusted).map_err(|_| Error::OutOfBounds {
        what: "adjusted size field",
        value: offset,
        len: buffer.len(),
    })?;
    write_i32(buffer, offset, adjusted);
    Ok(())
}

fn free_id(body: &[u8]) -> u32 {
    (0..u32::MAX)
        .map(|attempt| 2_146_000_000 - attempt * 2)
        .find(|&id| {
            let stack_name = format!("P9ItemStack_{}", id + 1);
            let has_stack = memmem::find(body, stack_name.as_bytes()).is_some();
            let instance_suffix = format!("_C_{id}\0");
            let has_instance = memmem::find(body, instance_suffix.as_bytes()).is_some();
            !has_stack && !has_instance
        })
        .expect("id space cannot be exhausted")
}

struct TemplateParts {
    class_path_start: usize,
    class_path_len: usize,
    instance_name_start: usize,
    instance_name_len: usize,
    stack_name_start: usize,
    stack_name_len: usize,
    s1_offset: usize,
    s2_offset: usize,
    s3_offset: usize,
    props_start: usize,
    props_len: usize,
}

fn parse_template(record: &[u8]) -> Option<TemplateParts> {
    let (script, after_script) = read_lstr(record, 0)?;
    if script != &STACK_SCRIPT[..STACK_SCRIPT.len() - 1] {
        return None;
    }
    let stack_name_start = after_script;
    let (stack_name, after_name) = read_lstr(record, stack_name_start)?;
    let stack_name_len = after_name - stack_name_start;
    if read_i32(record, after_name)? != 0 {
        return None;
    }
    let s1_offset = after_name + 4;

    let none_and_size = memmem::find(record, b"\x05\x00\x00\x00None\x00\x00\x00\x00\x00")?;
    let s2_offset = none_and_size + 13;
    let marker_start = s2_offset + 4;
    if record.get(marker_start..marker_start + 4)? != EMBEDDED_MARKER {
        return None;
    }
    if read_i32(record, marker_start + 4)? != 0 {
        return None;
    }
    let class_path_start = marker_start + 8;
    let (class_path, after_class) = read_lstr(record, class_path_start)?;
    if !class_path.starts_with(ITEM_PATH_PREFIX.as_bytes()) {
        return None;
    }
    let class_path_len = after_class - class_path_start;
    let instance_name_start = after_class;
    let (_instance, after_instance) = read_lstr(record, instance_name_start)?;
    let instance_name_len = after_instance - instance_name_start;
    if read_i32(record, after_instance)? != 0 {
        return None;
    }
    let s3_offset = after_instance + 4;
    let props_len = usize::try_from(read_i32(record, s3_offset)?).ok()?;
    let props_start = s3_offset + 4;
    record.get(props_start..props_start + props_len)?;
    let _ = stack_name;
    Some(TemplateParts {
        class_path_start,
        class_path_len,
        instance_name_start,
        instance_name_len,
        stack_name_start,
        stack_name_len,
        s1_offset,
        s2_offset,
        s3_offset,
        props_start,
        props_len,
    })
}

/// Insert a new stack of `count` x `item` by cloning the first existing
/// stack record. Returns the rebuilt body.
///
/// # Errors
///
/// Returns an error if the inventory cannot be located, the save contains
/// no stack to use as a template, or a size field cannot be adjusted.
///
/// # Panics
///
/// Panics only if internal length arithmetic overflows `i64`/`i32`, which
/// would require a multi-gigabyte save body; real bodies are a few MB.
pub fn add_stack(
    body: &[u8],
    version: ArchiveVersion,
    item: &ItemClass,
    count: i32,
) -> Result<Vec<u8>> {
    match version {
        ArchiveVersion::V3 => {}
        ArchiveVersion::V2 | ArchiveVersion::Other(_) => {
            return Err(Error::UnsupportedArchiveVersion(match version {
                ArchiveVersion::V2 => 2,
                ArchiveVersion::Other(value) => value,
                ArchiveVersion::V3 => unreachable!(),
            }));
        }
    }
    let Inventory {
        stacks,
        list_size_offset,
        stack_count_offset,
        subsystem_size_offset,
        first_record_start,
    } = inventory(body, version)?;

    let Some(template_stack) = stacks.first() else {
        return Err(structure_error("no template stack available"));
    };
    let template = &body[template_stack.record_start..template_stack.record_end];
    let parts =
        parse_template(template).ok_or_else(|| structure_error("template stack did not parse"))?;
    let TemplateParts {
        class_path_start,
        class_path_len,
        instance_name_start,
        instance_name_len,
        stack_name_start,
        stack_name_len,
        s1_offset,
        s2_offset,
        s3_offset,
        props_start,
        props_len,
    } = parts;

    let id = free_id(body);
    let ItemClass(stem) = item;
    let class_path = lstr(format!("{ITEM_PATH_PREFIX}BP_Item_{stem}.BP_Item_{stem}_C").as_bytes());
    let instance_name = lstr(format!("BP_Item_{stem}_C_{id}").as_bytes());
    let stack_name = lstr(format!("P9ItemStack_{}", id + 1).as_bytes());

    let delta_props = i64::try_from(EMPTY_PROPS.len()).expect("small")
        - i64::try_from(props_len).expect("record-sized");
    let delta_class = i64::try_from(class_path.len()).expect("small")
        - i64::try_from(class_path_len).expect("small");
    let delta_instance = i64::try_from(instance_name.len()).expect("small")
        - i64::try_from(instance_name_len).expect("small");
    let delta_inner = delta_props + delta_class + delta_instance;
    let delta_stack_name = i64::try_from(stack_name.len()).expect("small")
        - i64::try_from(stack_name_len).expect("small");

    let record = splice(template, props_start, props_len, EMPTY_PROPS);
    let record = splice(
        &record,
        instance_name_start,
        instance_name_len,
        &instance_name,
    );
    let record = splice(&record, class_path_start, class_path_len, &class_path);
    let mut record = splice(&record, stack_name_start, stack_name_len, &stack_name);

    let shift_by_name = |offset: usize| -> usize {
        usize::try_from(i64::try_from(offset).expect("offset") + delta_stack_name)
            .expect("offsets stay positive")
    };
    adjust_i32(&mut record, shift_by_name(s1_offset), delta_inner)?;
    adjust_i32(&mut record, shift_by_name(s2_offset), delta_inner)?;
    let s3_shifted = usize::try_from(
        i64::try_from(s3_offset).expect("offset") + delta_stack_name + delta_class + delta_instance,
    )
    .expect("offsets stay positive");
    write_i32(
        &mut record,
        s3_shifted,
        i32::try_from(EMPTY_PROPS.len()).expect("small"),
    );

    let count_position = memmem::find(&record, COUNT_PROP_V3)
        .ok_or_else(|| structure_error("cloned record lost Count property"))?
        + COUNT_PROP_V3.len();
    write_i32(&mut record, count_position, count);

    let record_len = i64::try_from(record.len()).expect("record-sized");
    let mut out = splice(body, first_record_start, 0, &record);
    adjust_i32(&mut out, list_size_offset, record_len)?;
    adjust_i32(&mut out, stack_count_offset, 1)?;
    adjust_i32(&mut out, subsystem_size_offset, record_len)?;
    adjust_i32(&mut out, 0, record_len)?;
    Ok(out)
}
