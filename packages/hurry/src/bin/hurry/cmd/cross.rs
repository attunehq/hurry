use color_eyre::Result;
use hurry::cross;

/// Execute a cross command by passing through all arguments.
pub async fn exec(arguments: Vec<String>) -> Result<()> {
    let Some((command, options)) = arguments.split_first() else {
        return cross::invoke_plain(Vec::<String>::new()).await;
    };

    cross::invoke(command, options).await
}
