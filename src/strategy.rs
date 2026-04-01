use crate::context::Context;

#[async_trait::async_trait(?Send)]
pub trait Strategy {
    async fn next(&mut self, cx: &Context) -> anyhow::Result<()>;
}

#[async_trait::async_trait(?Send)]
impl<T> Strategy for T
where
    T: AsyncFn(&Context) -> anyhow::Result<()>,
{
    async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
        self(cx).await
    }
}
