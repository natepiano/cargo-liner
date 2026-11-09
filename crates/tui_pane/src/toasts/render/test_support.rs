use crate::AppContext;
use crate::Framework;

pub(super) struct TestApp {
    framework: Framework<Self>,
}

impl AppContext for TestApp {
    type AppPaneId = ();
    type ToastAction = ();

    fn framework(&self) -> &Framework<Self> { &self.framework }

    fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
}
