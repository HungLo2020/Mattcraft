//! Explicit resource sampling, independent of backend state and texture storage.
use super::{SamplerAddressMode, SamplerDesc, SamplerFilter};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextureSampling {
    pub filter: SamplerFilter,
    pub address: SamplerAddressMode,
}

impl TextureSampling {
    pub(crate) fn decode(filter: u32, address: u32) -> super::GalResult<Option<Self>> {
        if filter == 0 && address == 0 {
            return Ok(None);
        }
        if !(1..=2).contains(&filter) || !(1..=2).contains(&address) {
            return Err(super::GalError::invalid_argument(
                "invalid explicit texture sampling descriptor",
            ));
        }
        Ok(Some(Self::from_texture_metadata(filter == 2, address == 2)))
    }
    pub(crate) fn from_texture_metadata(blur: bool, clamp: bool) -> Self {
        Self {
            filter: if blur {
                SamplerFilter::Linear
            } else {
                SamplerFilter::Nearest
            },
            address: if clamp {
                SamplerAddressMode::ClampToEdge
            } else {
                SamplerAddressMode::Repeat
            },
        }
    }

    pub(crate) fn descriptor(self, label: String, mip_filter: SamplerFilter) -> SamplerDesc {
        SamplerDesc {
            label,
            min_filter: self.filter,
            mag_filter: self.filter,
            mip_filter,
            address_u: self.address,
            address_v: self.address,
            address_w: self.address,
            comparison: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_rejects_unknown_or_partial_sampling_state() {
        assert_eq!(TextureSampling::decode(0, 0).unwrap(), None);
        for (filter, address) in [(0, 1), (1, 0), (3, 1), (1, 3), (u32::MAX, 2)] {
            assert!(TextureSampling::decode(filter, address).is_err());
        }
        assert_eq!(
            TextureSampling::decode(2, 2).unwrap(),
            Some(TextureSampling::from_texture_metadata(true, true))
        );
    }

    #[test]
    fn texture_metadata_controls_explicit_filter_and_address_independently() {
        for blur in [false, true] {
            for clamp in [false, true] {
                let sampling = TextureSampling::from_texture_metadata(blur, clamp);
                let descriptor = sampling.descriptor("sky".into(), SamplerFilter::Nearest);
                let filter = if blur {
                    SamplerFilter::Linear
                } else {
                    SamplerFilter::Nearest
                };
                let address = if clamp {
                    SamplerAddressMode::ClampToEdge
                } else {
                    SamplerAddressMode::Repeat
                };
                assert_eq!(descriptor.min_filter, filter);
                assert_eq!(descriptor.mag_filter, filter);
                assert_eq!(descriptor.address_u, address);
                assert_eq!(descriptor.address_v, address);
                assert_eq!(descriptor.address_w, address);
                assert_eq!(descriptor.mip_filter, SamplerFilter::Nearest);
                assert_eq!(descriptor.comparison, None);
            }
        }
    }
}
