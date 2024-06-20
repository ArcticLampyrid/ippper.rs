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

pub fn remove_ipp_attribute(
    r: &mut IppAttributes,
    tag: DelimiterTag,
    name: &str,
) -> Option<IppValue> {
    r.groups_mut()
        .iter_mut()
        .filter(|g| g.tag() == tag)
        .find_map(|g| g.attributes_mut().remove(name))
        .map(|a| a.into_value())
}
