package net.vulkanic.bridge;

import java.lang.foreign.Arena;
import java.lang.foreign.ValueLayout;
import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.*;
import static net.vulkanic.bridge.VulkanicGalBridge.*;

class EngineGlobalsEncodingTest {
    @Test
    void actualNativeLayoutReceivesRawCopiedSemanticsWithoutTouchingEarlierFields() {
        try (var arena = Arena.ofConfined()) {
            var layout = Struct.WHOLE_FRAME_SUBMIT;
            var request = layout.allocate(arena);
            request.fill((byte) 0x5a);
            var prefix = request.asSlice(0, layout.offset(37)).toArray(ValueLayout.JAVA_BYTE);
            var values = new EngineGlobalsRecord(854, 480, Long.MAX_VALUE - 17, 0.125f, 0.75f, 7);
            writeEngineGlobals(request, values);
            assertArrayEquals(prefix, request.asSlice(0, layout.offset(37)).toArray(ValueLayout.JAVA_BYTE));
            assertEquals(1, request.get(ValueLayout.JAVA_INT, layout.offset(37)));
            assertEquals(854, request.get(ValueLayout.JAVA_INT, layout.offset(38)));
            assertEquals(480, request.get(ValueLayout.JAVA_INT, layout.offset(39)));
            assertEquals(Long.MAX_VALUE - 17, request.get(ValueLayout.JAVA_LONG, layout.offset(40)));
            assertEquals(0.125f, request.get(ValueLayout.JAVA_FLOAT, layout.offset(41)));
            assertEquals(0.75f, request.get(ValueLayout.JAVA_FLOAT, layout.offset(42)));
            assertEquals(7, request.get(ValueLayout.JAVA_INT, layout.offset(43)));
            writeEngineGlobals(request, null);
            assertEquals(0, request.get(ValueLayout.JAVA_INT, layout.offset(37)));
            assertArrayEquals(prefix, request.asSlice(0, layout.offset(37)).toArray(ValueLayout.JAVA_BYTE));
        }
    }
}
