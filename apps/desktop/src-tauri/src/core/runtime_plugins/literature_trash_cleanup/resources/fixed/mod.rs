use super::WriteSpec;
mod part_0;
mod part_1;
mod part_2;
mod part_3;
mod part_4;
mod part_5;
pub(super) fn specs() -> impl Iterator<Item = &'static WriteSpec> {
    part_0::SPECS
        .iter()
        .chain(part_1::SPECS)
        .chain(part_2::SPECS)
        .chain(part_3::SPECS)
        .chain(part_4::SPECS)
        .chain(part_5::SPECS)
}
