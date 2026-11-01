import type { CSSProperties } from "react";
import type { VnPresentedView } from "@rezics/narrata/vn/player";

export function PlayerStage({ screen }: { screen: VnPresentedView | undefined }) {
  if (!screen) return <div className="scene-stage is-empty" aria-hidden="true" />;
  const background = screen.assets.find(asset => asset.slot.role === "background");
  const layers = screen.assets.filter(asset => asset.slot.role === "character").sort((left, right) => {
    const a = left.slot.role === "character" ? left.slot.order : 0;
    const b = right.slot.role === "character" ? right.slot.order : 0;
    return a - b;
  });
  return (
    <div className="scene-stage" aria-hidden="true" data-testid="scene-stage">
      {background?.presentation.kind === "image" && (
        <img
          className="scene-background"
          src={background.presentation.source}
          alt=""
          data-background={background.media.key}
          data-variant={background.variant}
          draggable="false"
        />
      )}
      {layers.map(asset => {
        if (asset.presentation.kind !== "image" || asset.slot.role !== "character") return null;
        const name = asset.slot.id.toLowerCase().replace(/[^a-z0-9-]/g, "");
        const style: CSSProperties = { zIndex: asset.slot.order };
        return (
          <img
            className={`scene-character character-${name}`}
            src={asset.presentation.source}
            alt=""
            key={`${asset.slot.id}-${asset.media.provider}-${asset.media.key}-${asset.media.revision}`}
            data-layer={asset.slot.id}
            data-order={asset.slot.order}
            draggable="false"
            style={style}
          />
        );
      })}
    </div>
  );
}
