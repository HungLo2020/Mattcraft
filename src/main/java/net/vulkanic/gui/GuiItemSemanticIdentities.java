package net.vulkanic.gui;

import java.util.HashMap;
import java.util.Map;
import java.util.Objects;

/** Bounded names for source model identities. No pixels, GPU objects, or redraw decisions. */
final class GuiItemSemanticIdentities {
    private static final int LIMIT = 64;
    private static final Map<Object, Long> IDENTITIES = new HashMap<>();
    private static long nextIdentity = 1;

    private GuiItemSemanticIdentities() {}

    static synchronized long identity(Object immutableModelIdentity) {
        Objects.requireNonNull(immutableModelIdentity);
        Long existing = IDENTITIES.get(immutableModelIdentity);
        if (existing != null) return existing;
        if (IDENTITIES.size() >= LIMIT || nextIdentity == Long.MAX_VALUE)
            throw new IllegalStateException("GUI semantic item identity capacity exceeded");
        long identity = nextIdentity++;
        IDENTITIES.put(immutableModelIdentity, identity);
        return identity;
    }

    static synchronized void clear() {
        IDENTITIES.clear(); // Never reuse names while old submissions might exist.
    }
}
