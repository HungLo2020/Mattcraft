package net.minecraft.client.dev;

import java.util.concurrent.CompletableFuture;
import net.minecraft.client.Minecraft;

/** Opt-in normal resource reload, with completion and presentation evidence. */
public final class GraphicsAuditResourceReload {
    private static final boolean ENABLED = "true".equalsIgnoreCase(System.getenv("MATTMC_CAPTURE_MENU_RELOAD"));
    private static final Sequence ACTIVE = new Sequence();
    private static final boolean WORLD_ENABLED =
        Boolean.getBoolean("mattmc.dev.deterministicCameraCapture.resourceReload");
    private static final WorldSequence WORLD = new WorldSequence();
    private static java.util.List<String> selectedBefore = java.util.List.of();
    private static java.util.List<String> selectedAtCapture = java.util.List.of();
    private static CompletableFuture<Void> reload;
    private static int observedFrames;
    public static void observe(Minecraft minecraft) {
        if (!ENABLED || observedFrames++ >= 1200 || observedFrames % 60 != 1) return;
        System.out.println("[MattMC graphics audit] reload-observation requested=" + (reload != null)
            + " done=" + (reload != null && reload.isDone()) + " complete=" + ACTIVE.complete()
            + " screen=" + (minecraft.screen == null ? "null" : minecraft.screen.getClass().getSimpleName())
            + " overlay=" + (minecraft.getOverlay() == null ? "null" : minecraft.getOverlay().getClass().getSimpleName())
            + " titleFadeReady=" + net.minecraft.client.gui.screens.TitleScreen.graphicsAuditTitleScreenFadeComplete());
    }
    private GraphicsAuditResourceReload() {}

    /** Separate from menu reloads; no renderer state or backend identity is consulted. */
    public static final class WorldSequence {
        private CompletableFuture<Void> future;
        private long lastFrame = Long.MIN_VALUE;
        private int presentations;
        public boolean afterPresentation(long frame, boolean overlay,
                java.util.function.Supplier<CompletableFuture<Void>> beginReload) {
            if (future == null) {
                future = java.util.Objects.requireNonNull(beginReload.get());
                lastFrame = frame;
                return false;
            }
            if (future.isDone()) future.join();
            if (!future.isDone() || overlay) {
                presentations = 0;
                lastFrame = frame;
                return false;
            }
            if (frame > lastFrame) {
                presentations = Math.min(2, presentations + 1);
                lastFrame = frame;
            }
            return presentations >= 2;
        }
        public com.google.gson.JsonObject receipt() {
            var state = new com.google.gson.JsonObject();
            state.addProperty("schema", "normal-world-resource-reload-v1");
            state.addProperty("requested", future != null);
            state.addProperty("futureComplete", future != null && future.isDone()
                && !future.isCompletedExceptionally() && !future.isCancelled());
            state.addProperty("complete", presentations >= 2);
            state.addProperty("presentations", presentations);
            return state;
        }
    }

    public static boolean prepareWorldCapture(Minecraft minecraft, long frame) {
        if (!WORLD_ENABLED) return true;
        boolean ready = WORLD.afterPresentation(frame, minecraft.getOverlay() != null, () -> {
            var repository = minecraft.getResourcePackRepository();
            selectedBefore = java.util.List.copyOf(repository.getSelectedIds());
            String remove = System.getProperty("mattmc.dev.deterministicCameraCapture.reloadRemovePack", "");
            if (!remove.isEmpty()) repository.setSelected(withoutSelectedPack(selectedBefore, remove));
            return minecraft.reloadResourcePacks();
        });
        selectedAtCapture = java.util.List.copyOf(minecraft.getResourcePackRepository().getSelectedIds());
        return ready;
    }

    public static java.util.List<String> withoutSelectedPack(java.util.List<String> selected, String remove) {
        if (remove.isEmpty() || !selected.contains(remove))
            throw new IllegalArgumentException("reload fixture pack is not selected: " + remove);
        var remaining = selected.stream().filter(pack -> !pack.equals(remove)).toList();
        if (remaining.isEmpty()) throw new IllegalArgumentException("reload fixture requires a remaining pack");
        return remaining;
    }

    public static String worldReceipt() {
        if (!WORLD_ENABLED) return "null";
        var receipt = WORLD.receipt();
        var before = new com.google.gson.JsonArray();
        selectedBefore.forEach(before::add);
        var after = new com.google.gson.JsonArray();
        selectedAtCapture.forEach(after::add);
        receipt.add("selectedBefore", before);
        receipt.add("selectedAtCapture", after);
        return receipt.toString();
    }

    public static final class Sequence {
        public enum Action { RELOAD, WAIT, CAPTURE }
        private boolean requested;
        private int presentations;
        public Action afterPresentation(boolean done, boolean overlayPresent) {
            if (!requested) { requested = true; return Action.RELOAD; }
            if (!done || overlayPresent) { presentations = 0; return Action.WAIT; }
            return ++presentations >= 2 ? Action.CAPTURE : Action.WAIT;
        }
        public boolean complete() { return requested && presentations >= 2; }
    }

    public static boolean prepareCapture(Minecraft minecraft) {
        if (!ENABLED) return true;
        if (reload != null && reload.isDone()) reload.join(); // failures cannot become successful evidence
        var action = ACTIVE.afterPresentation(reload != null && reload.isDone(), minecraft.getOverlay() != null);
        if (action == Sequence.Action.RELOAD) reload = minecraft.reloadResourcePacks();
        return action == Sequence.Action.CAPTURE;
    }

    public static void captureState(com.google.gson.JsonObject state) {
        if (!ENABLED) return;
        state.addProperty("resourceReloadFixture", "normal-reload-complete-presented-v1");
        state.addProperty("resourceReloadComplete", ACTIVE.complete());
    }
}
