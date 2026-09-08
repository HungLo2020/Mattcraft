package net.vulkanic.gui;

import java.util.Objects;

/** Immutable semantic sources; an atlas reference never masquerades as pixel data. */
sealed interface GuiItemTextureSource {
    long assetId();
    void stage();

    record Raw(RustGalGuiRawImageAssets.Asset asset) implements GuiItemTextureSource {
        public Raw { Objects.requireNonNull(asset); }
        public long assetId() { return asset.assetId(); }
        public void stage() { RustGalGuiRawImageAssets.stage(asset); }
    }

    record Atlas(GuiAtlasRegion region) implements GuiItemTextureSource {
        public Atlas { Objects.requireNonNull(region); }
        public long assetId() { return region.assetId(); }
        public void stage() { RustGalFrameCoordinator.stageGuiAtlasReference(region); }
    }
}
