use clap::Args;

#[derive(Args, Debug)]
pub(crate) struct JsonOnlyArgs {
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Args, Debug)]
pub(crate) struct IdArg {
    pub(crate) id: String,
}
