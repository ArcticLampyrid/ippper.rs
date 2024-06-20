use ipp::{attribute::IppAttributes, model::DelimiterTag, value::IppValue};

pub fn get_ipp_attribute<'a>(
    r: &'a IppAttributes,
    tag: DelimiterTag,
    name: &str,
) -> Option<&'a IppValue> {
    r.groups_of(tag)
        .find_map(|g| g.attributes().get(name))
        .map(|a| a.value())
}
