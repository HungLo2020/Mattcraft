package net.vulkanic.gui;

import java.util.List;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class GuiItemSemanticIdentitiesTest {
    @Test void equalityNotHashCollisionsNamesModelsAndReloadNeverReusesNames() {
        GuiItemSemanticIdentities.clear();
        try {
            long first = GuiItemSemanticIdentities.identity(List.of("Aa"));
            assertEquals(first, GuiItemSemanticIdentities.identity(List.of("Aa")));
            assertEquals("Aa".hashCode(), "BB".hashCode());
            assertNotEquals(first, GuiItemSemanticIdentities.identity(List.of("BB")));
            GuiItemSemanticIdentities.clear();
            assertNotEquals(first, GuiItemSemanticIdentities.identity(List.of("Aa")));
        } finally { GuiItemSemanticIdentities.clear(); }
    }

    @Test void capacityRejectsBeforePublicationAndExistingNamesRemainUsable() {
        GuiItemSemanticIdentities.clear();
        try {
            long first = GuiItemSemanticIdentities.identity(List.of(0));
            for (int i = 1; i < 64; i++) GuiItemSemanticIdentities.identity(List.of(i));
            assertThrows(IllegalStateException.class, () -> GuiItemSemanticIdentities.identity(List.of(64)));
            assertEquals(first, GuiItemSemanticIdentities.identity(List.of(0)));
        } finally { GuiItemSemanticIdentities.clear(); }
    }
}
