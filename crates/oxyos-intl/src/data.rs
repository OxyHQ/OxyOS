//! The CLDR data compiled in for dates and amounts: the Gregorian calendar's names and patterns,
//! and the currency and percent data of ICU4X's experimental components. ICU4X's own compiled
//! data for those crates would bring all of theirs into the library (every calendar, time zone
//! names, units, display names, transliteration...), whether used or not; a host that links
//! without `--gc-sections` would carry it all.
//!
//! Locales, decimals and plural rules come from their own crates' compiled data, which is small.
//! Of what this data adds to a linked program (4.5 MB), currencies' names by plural form
//! (`currencyDisplay: "name"`, every currency in every locale) are 3 MB.

use icu_provider::prelude::*;

/// The data, for ICU4X's `_unstable` constructors and loads (which this version's data matches;
/// Cargo.lock keeps the two together).
pub(crate) struct Baked;

#[allow(unused_imports)]
const _: () = {
    use icu_datetime_data::*;
    use icu_experimental_data::*;
    // The paths the data names.
    mod icu {
        pub use icu_datetime as datetime;
        pub use icu_experimental as experimental;
        pub use icu_locale as locale;
        pub use icu_plurals as plurals;
    }
    icu_datetime_data::make_provider!(Baked);
    impl_datetime_patterns_glue_v1!(Baked);
    impl_datetime_patterns_time_v1!(Baked);
    impl_datetime_patterns_date_gregorian_v1!(Baked);
    impl_datetime_names_weekday_v1!(Baked);
    impl_datetime_names_dayperiod_v1!(Baked);
    impl_datetime_names_year_gregorian_v1!(Baked);
    impl_datetime_names_month_gregorian_v1!(Baked);
    impl_currency_essentials_v1!(Baked);
    impl_currency_extended_data_v1!(Baked);
    impl_currency_patterns_data_v1!(Baked);
    impl_percent_essentials_v1!(Baked);
};

/// A date's digits are a decimal formatter's.
macro_rules! decimal {
    ($($marker:ident),*) => {$(
        impl DataProvider<icu_decimal::provider::$marker> for Baked {
            fn load(
                &self,
                request: DataRequest,
            ) -> Result<DataResponse<icu_decimal::provider::$marker>, DataError> {
                icu_decimal::provider::Baked.load(request)
            }
        }
    )*};
}

decimal!(DecimalSymbolsV1, DecimalDigitsV1);
