use schemaform::InstanceIdentity;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RenderEvent {
    RendererEntered,
    Mounted,
    Dropped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RenderNodeKind {
    Control,
    StaticLayout,
    Collection,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RenderObservation {
    pub event: RenderEvent,
    pub identity: InstanceIdentity,
    pub node_kind: RenderNodeKind,
    pub dom_id: String,
}

pub trait RenderObserver {
    fn observe(&self, observation: RenderObservation);
}
