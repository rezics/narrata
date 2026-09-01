use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use crate::{
    ActorId, AudioChannelId, EntityId, InteractionId, LayerId,
    codec::{CborReader, CborWriter, DecodeError},
    limits::DecodeLimits,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResumeSupport {
    Restart,
    Seek,
    BestEffort,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LayerState {
    pub id: LayerId,
    pub asset: Option<EntityId>,
    pub visible: bool,
    pub z_index: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActorState {
    pub id: ActorId,
    pub asset: EntityId,
    pub layer: LayerId,
    pub visible: bool,
    pub x_milli: i64,
    pub y_milli: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CameraState {
    pub x_milli: i64,
    pub y_milli: i64,
    pub zoom_milli: u32,
}

impl Default for CameraState {
    fn default() -> Self {
        Self {
            x_milli: 0,
            y_milli: 0,
            zoom_milli: 1_000,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioState {
    pub asset: EntityId,
    pub playing: bool,
    pub looping: bool,
    pub position_millis: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SceneInteractionView {
    Dialogue(InteractionId),
    Choice(InteractionId),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SceneState {
    layers: Vec<LayerState>,
    actors: BTreeMap<ActorId, ActorState>,
    camera: CameraState,
    audio_channels: BTreeMap<AudioChannelId, AudioState>,
    interaction: Option<SceneInteractionView>,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum SceneError {
    #[error("scene contains a duplicate layer ID")]
    DuplicateLayer,
    #[error("actor key does not match ActorState.id")]
    ActorKeyMismatch,
    #[error("actor refers to a missing layer")]
    MissingActorLayer,
    #[error("camera zoom must be greater than zero")]
    InvalidCameraZoom,
}

impl SceneState {
    pub fn checked(
        layers: Vec<LayerState>,
        actors: BTreeMap<ActorId, ActorState>,
        camera: CameraState,
        audio_channels: BTreeMap<AudioChannelId, AudioState>,
        interaction: Option<SceneInteractionView>,
    ) -> Result<Self, SceneError> {
        let layer_ids = layers.iter().map(|layer| layer.id).collect::<BTreeSet<_>>();
        if layer_ids.len() != layers.len() {
            return Err(SceneError::DuplicateLayer);
        }
        if actors.iter().any(|(id, actor)| id != &actor.id) {
            return Err(SceneError::ActorKeyMismatch);
        }
        if actors
            .values()
            .any(|actor| !layer_ids.contains(&actor.layer))
        {
            return Err(SceneError::MissingActorLayer);
        }
        if camera.zoom_milli == 0 {
            return Err(SceneError::InvalidCameraZoom);
        }
        Ok(Self {
            layers,
            actors,
            camera,
            audio_channels,
            interaction,
        })
    }

    pub fn layers(&self) -> &[LayerState] {
        &self.layers
    }

    pub fn actors(&self) -> &BTreeMap<ActorId, ActorState> {
        &self.actors
    }

    pub const fn camera(&self) -> CameraState {
        self.camera
    }

    pub fn audio_channels(&self) -> &BTreeMap<AudioChannelId, AudioState> {
        &self.audio_channels
    }

    pub const fn interaction(&self) -> Option<SceneInteractionView> {
        self.interaction
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconcileScene {
    pub target: SceneState,
}

pub(crate) fn encode_scene(writer: &mut CborWriter, scene: &SceneState) {
    writer.array(5);
    writer.array(scene.layers.len() as u64);
    for layer in &scene.layers {
        writer.array(4);
        writer.bytes(layer.id.as_bytes());
        match layer.asset {
            Some(asset) => writer.bytes(asset.as_bytes()),
            None => writer.null(),
        }
        writer.boolean(layer.visible);
        writer.signed(layer.z_index);
    }
    writer.map(scene.actors.len() as u64);
    for (id, actor) in &scene.actors {
        writer.bytes(id.as_bytes());
        writer.array(5);
        writer.bytes(actor.asset.as_bytes());
        writer.bytes(actor.layer.as_bytes());
        writer.boolean(actor.visible);
        writer.signed(actor.x_milli);
        writer.signed(actor.y_milli);
    }
    writer.array(3);
    writer.signed(scene.camera.x_milli);
    writer.signed(scene.camera.y_milli);
    writer.unsigned(u64::from(scene.camera.zoom_milli));
    writer.map(scene.audio_channels.len() as u64);
    for (id, audio) in &scene.audio_channels {
        writer.bytes(id.as_bytes());
        writer.array(4);
        writer.bytes(audio.asset.as_bytes());
        writer.boolean(audio.playing);
        writer.boolean(audio.looping);
        match audio.position_millis {
            Some(position) => writer.unsigned(position),
            None => writer.null(),
        }
    }
    match scene.interaction {
        Some(SceneInteractionView::Dialogue(id)) => {
            writer.array(2);
            writer.unsigned(0);
            writer.bytes(id.as_bytes());
        }
        Some(SceneInteractionView::Choice(id)) => {
            writer.array(2);
            writer.unsigned(1);
            writer.bytes(id.as_bytes());
        }
        None => writer.null(),
    }
}

pub(crate) fn decode_scene(
    reader: &mut CborReader<'_>,
    limits: &DecodeLimits,
) -> Result<SceneState, DecodeError> {
    exact_array(reader, 5, "SceneState")?;
    let layer_len = bounded(reader.array_len()?, limits, "scene layers")?;
    let mut layers = Vec::with_capacity(to_usize(layer_len)?);
    for _ in 0..layer_len {
        exact_array(reader, 4, "scene layer")?;
        layers.push(LayerState {
            id: LayerId::from_bytes(reader.bytes_exact::<16>()?),
            asset: reader
                .optional(|reader| reader.bytes_exact::<16>().map(EntityId::from_bytes))?,
            visible: reader.boolean()?,
            z_index: reader.signed()?,
        });
    }
    let actor_len = bounded(reader.map_len()?, limits, "scene actors")?;
    let mut actors = BTreeMap::new();
    let mut previous_actor = None;
    for _ in 0..actor_len {
        let raw = reader.bytes_exact::<16>()?;
        ordered(previous_actor, raw, "scene actor order")?;
        previous_actor = Some(raw);
        exact_array(reader, 5, "scene actor")?;
        let id = ActorId::from_bytes(raw);
        actors.insert(
            id,
            ActorState {
                id,
                asset: EntityId::from_bytes(reader.bytes_exact::<16>()?),
                layer: LayerId::from_bytes(reader.bytes_exact::<16>()?),
                visible: reader.boolean()?,
                x_milli: reader.signed()?,
                y_milli: reader.signed()?,
            },
        );
    }
    exact_array(reader, 3, "scene camera")?;
    let camera = CameraState {
        x_milli: reader.signed()?,
        y_milli: reader.signed()?,
        zoom_milli: u32::try_from(reader.unsigned()?).map_err(|_| DecodeError::IntegerOverflow)?,
    };
    let audio_len = bounded(reader.map_len()?, limits, "scene audio channels")?;
    let mut audio_channels = BTreeMap::new();
    let mut previous_audio = None;
    for _ in 0..audio_len {
        let raw = reader.bytes_exact::<16>()?;
        ordered(previous_audio, raw, "scene audio order")?;
        previous_audio = Some(raw);
        exact_array(reader, 4, "scene audio")?;
        audio_channels.insert(
            AudioChannelId::from_bytes(raw),
            AudioState {
                asset: EntityId::from_bytes(reader.bytes_exact::<16>()?),
                playing: reader.boolean()?,
                looping: reader.boolean()?,
                position_millis: reader.optional(CborReader::unsigned)?,
            },
        );
    }
    let interaction = reader.optional(|reader| {
        exact_array(reader, 2, "scene interaction")?;
        let tag = reader.unsigned()?;
        let id = InteractionId::from_bytes(reader.bytes_exact::<32>()?);
        match tag {
            0 => Ok(SceneInteractionView::Dialogue(id)),
            1 => Ok(SceneInteractionView::Choice(id)),
            _ => Err(DecodeError::Schema("scene interaction tag")),
        }
    })?;
    SceneState::checked(layers, actors, camera, audio_channels, interaction)
        .map_err(|_| DecodeError::Schema("invalid SceneState"))
}

fn exact_array(
    reader: &mut CborReader<'_>,
    expected: u64,
    label: &'static str,
) -> Result<(), DecodeError> {
    if reader.array_len()? == expected {
        Ok(())
    } else {
        Err(DecodeError::Schema(label))
    }
}

fn bounded(value: u64, limits: &DecodeLimits, label: &'static str) -> Result<u64, DecodeError> {
    if value <= limits.max_collection_items {
        Ok(value)
    } else {
        Err(DecodeError::Limit(label))
    }
}

fn to_usize(value: u64) -> Result<usize, DecodeError> {
    usize::try_from(value).map_err(|_| DecodeError::LengthOverflow)
}

fn ordered(
    previous: Option<[u8; 16]>,
    next: [u8; 16],
    label: &'static str,
) -> Result<(), DecodeError> {
    if previous.is_some_and(|previous| previous >= next) {
        Err(DecodeError::NonCanonical(label))
    } else {
        Ok(())
    }
}
