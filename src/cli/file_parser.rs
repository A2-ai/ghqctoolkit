use crate::issues::RelevantFile;
use clap::builder::TypedValueParser;
use clap::{Arg, Command, error::ErrorKind};

// Custom parser for clap
#[derive(Clone)]
pub struct RelevantFileParser;

impl TypedValueParser for RelevantFileParser {
    type Value = RelevantFile;

    fn parse_ref(
        &self,
        _cmd: &Command,
        arg: Option<&Arg>,
        value: &std::ffi::OsStr,
    ) -> Result<Self::Value, clap::Error> {
        let s = value.to_str().ok_or_else(|| {
            clap::Error::raw(
                ErrorKind::InvalidUtf8,
                "Invalid UTF-8 in file specification",
            )
        })?;

        s.parse().map_err(|_| {
            let mut err = clap::Error::new(ErrorKind::InvalidValue);
            if let Some(arg) = arg {
                err.insert(
                    clap::error::ContextKind::InvalidArg,
                    clap::error::ContextValue::String(arg.to_string()),
                );
            }
            err.insert(
                clap::error::ContextKind::InvalidValue,
                clap::error::ContextValue::String(s.to_string()),
            );
            err.insert(
                clap::error::ContextKind::ValidValue,
                clap::error::ContextValue::String("name:path".to_string()),
            );
            err
        })
    }
}
