package net.minecraft.world.level.chunk.storage;

import net.minecraft.SharedConstants;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.NbtAccounter;
import net.minecraft.nbt.NbtIo;
import net.minecraft.nbt.NativeNbtTestAccess;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.ChunkPos;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.io.ByteArrayOutputStream;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

class NativeRegionFileWriterTest {
	private static final RegionStorageInfo STORAGE_INFO = new RegionStorageInfo("test", null, "region-writer-test");
	private static final int SECTOR_BYTES = 4096;
	private static final int HEADER_BYTES = SECTOR_BYTES * 2;
	private static final int EXTERNAL_FLAG = 0x80;

	@BeforeAll
	static void bootstrapMinecraft() {
		SharedConstants.tryDetectVersion();
		Bootstrap.bootStrap();
	}

	@Test
	void rustWrittenRegionsAreReadableByJavaForAllCompressionTypes(@TempDir Path tempDir) throws IOException {
		List<RegionFileVersion> versions = List.of(
			RegionFileVersion.VERSION_GZIP,
			RegionFileVersion.VERSION_DEFLATE,
			RegionFileVersion.VERSION_NONE,
			RegionFileVersion.VERSION_LZ4
		);
		ChunkPos pos = new ChunkPos(0, 0);

		for (RegionFileVersion version : versions) {
			Path regionPath = tempDir.resolve("rust-format-" + version.getId()).resolve("r.0.0.mca");
			Files.createDirectories(regionPath.getParent());
			CompoundTag tag = sampleTag("rust-format-" + version.getId());
			byte[] payload = encodeWithJava(version, tag);

			NativeRegionFile.WriteResult write = NativeRegionFile.writePayload(regionPath, pos.x, pos.z, version.getId(), payload);

			assertEquals(NativeRegionFile.OK, write.status(), "format " + version.getId() + " " + write);
			assertTrue(write.present());
			assertEquals(version.getId(), write.compressionId());
			assertEquals(payload.length, write.payloadLength());

			RawPayload raw = readRawPayload(regionPath, pos);
			assertEquals(version.getId(), raw.compressionId());
			assertEquals(write.timestamp(), raw.timestamp());
			assertEquals(write.external(), raw.external());
			assertArrayEquals(payload, raw.payload());
			assertEquals(javaFingerprint(regionPath, pos), NativeRegionFile.readNbtFingerprint(regionPath, pos.x, pos.z).fingerprint());
		}
	}

	@Test
	void writerOverwritesGrowsShrinksDeletesReusesAndReopens(@TempDir Path tempDir) throws IOException {
		Path regionPath = tempDir.resolve("r.0.0.mca");
		ChunkPos a = new ChunkPos(0, 0);
		ChunkPos b = new ChunkPos(1, 0);
		byte[] smallA = encodeWithJava(RegionFileVersion.VERSION_NONE, sampleTag("small-a"));
		byte[] smallB = encodeWithJava(RegionFileVersion.VERSION_NONE, sampleTag("small-b"));
		byte[] largeA = encodeWithJava(RegionFileVersion.VERSION_NONE, largeTag("large-a", 9000));

		NativeRegionFile.WriteResult first = NativeRegionFile.writePayload(regionPath, a.x, a.z, RegionFileVersion.VERSION_NONE.getId(), smallA);
		NativeRegionFile.WriteResult second = NativeRegionFile.writePayload(regionPath, b.x, b.z, RegionFileVersion.VERSION_NONE.getId(), smallB);
		assertEquals(2, first.sectorOffset());
		assertEquals(3, second.sectorOffset());

		NativeRegionFile.WriteResult grown = NativeRegionFile.writePayload(regionPath, a.x, a.z, RegionFileVersion.VERSION_NONE.getId(), largeA);
		assertEquals(4, grown.sectorOffset());
		assertEquals(3, grown.sectorCount());

		NativeRegionFile.WriteResult shrunk = NativeRegionFile.writePayload(regionPath, a.x, a.z, RegionFileVersion.VERSION_NONE.getId(), smallA);
		assertEquals(2, shrunk.sectorOffset());
		assertEquals(1, shrunk.sectorCount());

		NativeRegionFile.WriteResult deleted = NativeRegionFile.deleteChunk(regionPath, b.x, b.z);
		assertEquals(NativeRegionFile.OK, deleted.status());
		assertFalse(deleted.present());
		try (RegionFile regionFile = new RegionFile(STORAGE_INFO, regionPath, tempDir, false)) {
			assertFalse(regionFile.hasChunk(b));
		}

		NativeRegionFile.WriteResult rewritten = NativeRegionFile.writePayload(regionPath, b.x, b.z, RegionFileVersion.VERSION_NONE.getId(), smallB);
		assertEquals(3, rewritten.sectorOffset());
		assertEquals(NativeRegionFile.OK, NativeRegionFile.flush(regionPath).status());

		try (RegionFile regionFile = new RegionFile(STORAGE_INFO, regionPath, tempDir, false)) {
			assertTrue(regionFile.hasChunk(a));
			assertTrue(regionFile.hasChunk(b));
		}
		assertEquals(javaFingerprint(regionPath, a), NativeRegionFile.readNbtFingerprint(regionPath, a.x, a.z).fingerprint());
		assertEquals(javaFingerprint(regionPath, b), NativeRegionFile.readNbtFingerprint(regionPath, b.x, b.z).fingerprint());
	}

	@Test
	void writerSwitchesBetweenInternalAndExternalStorage(@TempDir Path tempDir) throws IOException {
		Path regionPath = tempDir.resolve("r.0.0.mca");
		ChunkPos pos = new ChunkPos(0, 0);
		byte[] large = encodeWithJava(RegionFileVersion.VERSION_NONE, largeTag("external", 1_100_000));
		byte[] small = encodeWithJava(RegionFileVersion.VERSION_NONE, sampleTag("internal"));

		NativeRegionFile.WriteResult external = NativeRegionFile.writePayload(regionPath, pos.x, pos.z, RegionFileVersion.VERSION_NONE.getId(), large);

		assertEquals(NativeRegionFile.OK, external.status());
		assertTrue(external.external());
		assertEquals(1, external.sectorCount());
		assertTrue(Files.isRegularFile(tempDir.resolve("c.0.0.mcc")));
		assertEquals(javaFingerprint(regionPath, pos), NativeRegionFile.readNbtFingerprint(regionPath, pos.x, pos.z).fingerprint());

		NativeRegionFile.WriteResult internal = NativeRegionFile.writePayload(regionPath, pos.x, pos.z, RegionFileVersion.VERSION_NONE.getId(), small);

		assertEquals(NativeRegionFile.OK, internal.status());
		assertFalse(internal.external());
		assertFalse(Files.exists(tempDir.resolve("c.0.0.mcc")));
		assertEquals(javaFingerprint(regionPath, pos), NativeRegionFile.readNbtFingerprint(regionPath, pos.x, pos.z).fingerprint());
	}

	@Test
	void copiedRealRegionCanBeModifiedInTemporaryDirectory(@TempDir Path tempDir) throws IOException {
		Path source = Path.of("run", "saves", "mattmc-real-meshing-replay", "region", "r.0.0.mca");
		assumeTrue(Files.isRegularFile(source), "real-world meshing replay region is not available");
		Path copy = tempDir.resolve("r.0.0.mca");
		Files.copy(source, copy);
		ChunkPos pos = firstAbsentChunk(copy);
		if (pos == null) {
			pos = firstPresentChunk(copy);
		}
		assertNotNull(pos);
		byte[] payload = encodeWithJava(RegionFileVersion.VERSION_DEFLATE, sampleTag("real-copy-write"));

		NativeRegionFile.WriteResult write = NativeRegionFile.writePayload(copy, pos.x, pos.z, RegionFileVersion.VERSION_DEFLATE.getId(), payload);

		assertEquals(NativeRegionFile.OK, write.status());
		assertEquals(javaFingerprint(copy, pos), NativeRegionFile.readNbtFingerprint(copy, pos.x, pos.z).fingerprint());
	}

	@Test
	void writerRejectsMalformedInputAndMissingParents(@TempDir Path tempDir) throws IOException {
		Path regionPath = tempDir.resolve("r.0.0.mca");
		byte[] payload = encodeWithJava(RegionFileVersion.VERSION_NONE, sampleTag("bad"));

		NativeRegionFile.WriteResult invalidCompression = NativeRegionFile.writePayload(regionPath, 0, 0, 99, payload);
		assertEquals(-3, invalidCompression.status());

		NativeRegionFile.WriteResult missingParent = NativeRegionFile.writePayload(
			tempDir.resolve("missing").resolve("r.0.0.mca"),
			0,
			0,
			RegionFileVersion.VERSION_NONE.getId(),
			payload
		);
		assertEquals(-2, missingParent.status());
		assertFalse(Files.exists(tempDir.resolve("missing")));
	}

	@Test
	void regionFileDelegatesPayloadOperationsToRust(@TempDir Path tempDir) throws IOException {
		Path regionPath = tempDir.resolve("r.0.0.mca");
		ChunkPos pos = new ChunkPos(0, 0);
		CompoundTag firstTag = sampleTag("delegated-first");
		CompoundTag externalTag = largeTag("delegated-external", 1_100_000);

		try (RegionFile regionFile = new RegionFile(STORAGE_INFO, regionPath, tempDir, RegionFileVersion.VERSION_NONE, false);
			DataOutputStream output = regionFile.getChunkDataOutputStream(pos)) {
			NbtIo.write(firstTag, output);
		}

		NativeRegionFile.PayloadResult firstPayload = NativeRegionFile.readPayload(regionPath, pos.x, pos.z);
		assertEquals(NativeRegionFile.OK, firstPayload.result().status());
		assertTrue(firstPayload.result().present());
		assertFalse(firstPayload.result().external());
		assertEquals(RegionFileVersion.VERSION_NONE.getId(), firstPayload.result().compressionId());

		try (RegionFile regionFile = new RegionFile(STORAGE_INFO, regionPath, tempDir, RegionFileVersion.VERSION_NONE, false);
			InputStream input = regionFile.getChunkDataInputStream(pos)) {
			assertNotNull(input);
			assertEquals(firstTag, NbtIo.read(new DataInputStream(input), NbtAccounter.unlimitedHeap()));
			regionFile.flush();
		}

		try (RegionFile regionFile = new RegionFile(STORAGE_INFO, regionPath, tempDir, RegionFileVersion.VERSION_NONE, false);
			DataOutputStream output = regionFile.getChunkDataOutputStream(pos)) {
			NbtIo.write(externalTag, output);
		}

		NativeRegionFile.PayloadResult externalPayload = NativeRegionFile.readPayload(regionPath, pos.x, pos.z);
		assertEquals(NativeRegionFile.OK, externalPayload.result().status());
		assertTrue(externalPayload.result().present());
		assertTrue(externalPayload.result().external());
		assertTrue(Files.isRegularFile(tempDir.resolve("c.0.0.mcc")));

		try (RegionFile regionFile = new RegionFile(STORAGE_INFO, regionPath, tempDir, RegionFileVersion.VERSION_NONE, false)) {
			regionFile.clear(pos);
			regionFile.flush();
		}

		NativeRegionFile.PayloadResult deleted = NativeRegionFile.readPayload(regionPath, pos.x, pos.z);
		assertEquals(NativeRegionFile.OK, deleted.result().status());
		assertFalse(deleted.result().present());
		assertFalse(Files.exists(tempDir.resolve("c.0.0.mcc")));
	}

	@Test
	void regionFileStorageUsesNativeNbtTapePathForChunkReadWrite(@TempDir Path tempDir) throws IOException {
		ChunkPos pos = new ChunkPos(0, 0);
		CompoundTag tag = sampleTag("storage-tape-path");

		try (RegionFileStorage storage = new RegionFileStorage(STORAGE_INFO, tempDir, false)) {
			storage.write(pos, tag);
			assertEquals(tag, storage.read(pos));
		}

		Path regionPath = tempDir.resolve("r.0.0.mca");
		NativeRegionFile.PayloadResult payload = NativeRegionFile.readPayload(regionPath, pos.x, pos.z);
		assertEquals(NativeRegionFile.OK, payload.result().status());
		assertTrue(payload.result().present());
		assertEquals(RegionFileVersion.VERSION_DEFLATE.getId(), payload.result().compressionId());
		assertEquals(javaFingerprint(regionPath, pos), NativeRegionFile.readNbtFingerprint(regionPath, pos.x, pos.z).fingerprint());
	}

	@Test
	void regionFileUsesNativeHandleWithoutJavaMirror(@TempDir Path tempDir) throws IOException {
		Path missingPath = tempDir.resolve("missing").resolve("r.0.0.mca");
		Files.createDirectories(missingPath.getParent());
		ChunkPos pos = new ChunkPos(0, 0);

		try (RegionFile regionFile = new RegionFile(STORAGE_INFO, missingPath, missingPath.getParent(), RegionFileVersion.VERSION_DEFLATE, false)) {
			assertFalse(regionFile.hasChunk(pos));
			assertFalse(regionFile.doesChunkExist(pos));
		}
		assertTrue(Files.exists(missingPath), "persistent Rust region handles create the native-owned region header on open");
		assertEquals(8192L, Files.size(missingPath));

		Path regionPath = tempDir.resolve("r.0.0.mca");
		CompoundTag tag = sampleTag("rust-only-mirrorless");
		try (RegionFile regionFile = new RegionFile(STORAGE_INFO, regionPath, tempDir, RegionFileVersion.VERSION_DEFLATE, false);
			DataOutputStream output = regionFile.getChunkDataOutputStream(pos)) {
			NbtIo.write(tag, output);
		}

		try (RegionFile regionFile = new RegionFile(STORAGE_INFO, regionPath, tempDir, RegionFileVersion.VERSION_DEFLATE, false);
			InputStream input = regionFile.getChunkDataInputStream(pos)) {
			assertTrue(regionFile.hasChunk(pos));
			assertTrue(regionFile.doesChunkExist(pos));
			assertNotNull(input);
			assertEquals(tag, NbtIo.read(new DataInputStream(input), NbtAccounter.unlimitedHeap()));
		}

		try (RegionFile regionFile = new RegionFile(STORAGE_INFO, regionPath, tempDir, RegionFileVersion.VERSION_DEFLATE, false)) {
			regionFile.clear(pos);
			assertFalse(regionFile.hasChunk(pos));
			assertFalse(regionFile.doesChunkExist(pos));
		}

		try (RegionFile regionFile = new RegionFile(STORAGE_INFO, regionPath, tempDir, RegionFileVersion.VERSION_DEFLATE, false)) {
			assertFalse(regionFile.hasChunk(pos));
			assertFalse(regionFile.doesChunkExist(pos));
		}
	}

	@Test
	void regionFileHandlesAllCompressionTypesThroughNativeFacade(@TempDir Path tempDir) throws IOException {
		List<RegionFileVersion> versions = List.of(
			RegionFileVersion.VERSION_GZIP,
			RegionFileVersion.VERSION_DEFLATE,
			RegionFileVersion.VERSION_NONE,
			RegionFileVersion.VERSION_LZ4
		);

		for (RegionFileVersion version : versions) {
			Path regionPath = tempDir.resolve("delegated-format-" + version.getId()).resolve("r.0.0.mca");
			Files.createDirectories(regionPath.getParent());
			ChunkPos pos = new ChunkPos(version.getId(), 0);
			CompoundTag tag = sampleTag("delegated-format-" + version.getId());

			try (RegionFile regionFile = new RegionFile(STORAGE_INFO, regionPath, regionPath.getParent(), version, false);
				DataOutputStream output = regionFile.getChunkDataOutputStream(pos)) {
				NbtIo.write(tag, output);
			}

			try (RegionFile regionFile = new RegionFile(STORAGE_INFO, regionPath, regionPath.getParent(), version, false);
				InputStream input = regionFile.getChunkDataInputStream(pos)) {
				assertNotNull(input);
				assertEquals(tag, NbtIo.read(new DataInputStream(input), NbtAccounter.unlimitedHeap()));
			}

			NativeRegionFile.NbtResult fingerprint = NativeRegionFile.readNbtFingerprint(regionPath, pos.x, pos.z);
			assertEquals(NativeRegionFile.OK, fingerprint.status());
			assertTrue(fingerprint.present());
			assertEquals(version.getId(), fingerprint.compressionId());
			assertEquals(javaFingerprint(regionPath, pos), fingerprint.fingerprint());
		}
	}

	@Test
	void delegatedRegionFileSurvivesTwoCloseReopenCyclesOnCopiedRealRegion(@TempDir Path tempDir) throws IOException {
		Path source = Path.of("run", "saves", "mattmc-real-meshing-replay", "region", "r.0.0.mca");
		assumeTrue(Files.isRegularFile(source), "real-world meshing replay region is not available");
		Path copy = tempDir.resolve("r.0.0.mca");
		Files.copy(source, copy);
		ChunkPos readPos = firstPresentChunk(copy);
		assertNotNull(readPos);
		ChunkPos writePos = firstAbsentChunk(copy);
		if (writePos == null) {
			writePos = readPos;
		}
		CompoundTag written = sampleTag("delegated-real-copy");

		ChunkPos finalWritePos = writePos;
		try (RegionFile regionFile = new RegionFile(STORAGE_INFO, copy, tempDir, RegionFileVersion.VERSION_DEFLATE, false);
			InputStream input = regionFile.getChunkDataInputStream(readPos)) {
			assertNotNull(input);
			assertNotNull(NbtIo.read(new DataInputStream(input), NbtAccounter.unlimitedHeap()));
		}

		try (RegionFile regionFile = new RegionFile(STORAGE_INFO, copy, tempDir, RegionFileVersion.VERSION_DEFLATE, false);
			DataOutputStream output = regionFile.getChunkDataOutputStream(finalWritePos)) {
			NbtIo.write(written, output);
			regionFile.flush();
		}

		try (RegionFile regionFile = new RegionFile(STORAGE_INFO, copy, tempDir, RegionFileVersion.VERSION_DEFLATE, false);
			InputStream input = regionFile.getChunkDataInputStream(finalWritePos)) {
			assertNotNull(input);
			assertEquals(written, NbtIo.read(new DataInputStream(input), NbtAccounter.unlimitedHeap()));
			regionFile.flush();
		}

		assertEquals(javaFingerprint(copy, finalWritePos), NativeRegionFile.readNbtFingerprint(copy, finalWritePos.x, finalWritePos.z).fingerprint());
		try (var files = Files.list(tempDir)) {
			assertEquals(0, files.filter(path -> path.getFileName().toString().startsWith("tmp-mattmc-region-")).count());
		}
	}

	private static byte[] encodeWithJava(RegionFileVersion version, CompoundTag tag) throws IOException {
		ByteArrayOutputStream bytes = new ByteArrayOutputStream();
		try (DataOutputStream output = new DataOutputStream(version.wrap(bytes))) {
			NbtIo.write(tag, output);
		}
		return bytes.toByteArray();
	}

	private static CompoundTag sampleTag(String name) {
		CompoundTag tag = new CompoundTag();
		tag.putString("name", name);
		tag.putInt("x", 123);
		tag.putLong("seed", 987654321L);
		tag.putByteArray("bytes", new byte[]{1, 2, 3, 4});
		return tag;
	}

	private static CompoundTag largeTag(String name, int byteCount) {
		CompoundTag tag = sampleTag(name);
		tag.putByteArray("large", new byte[byteCount]);
		return tag;
	}

	private static long javaFingerprint(Path regionPath, ChunkPos pos) throws IOException {
		try (RegionFile regionFile = new RegionFile(STORAGE_INFO, regionPath, regionPath.getParent(), false);
			InputStream input = regionFile.getChunkDataInputStream(pos)) {
			assertNotNull(input);
			CompoundTag tag = NbtIo.read(new DataInputStream(input), NbtAccounter.unlimitedHeap());
			ByteArrayOutputStream raw = new ByteArrayOutputStream();
			try (DataOutputStream output = new DataOutputStream(raw)) {
				NbtIo.write(tag, output);
			}
			return NativeNbtTestAccess.fingerprintRaw(raw.toByteArray());
		}
	}

	private static ChunkPos firstPresentChunk(Path regionPath) throws IOException {
		byte[] bytes = Files.readAllBytes(regionPath);
		for (int i = 0; i < 1024; i++) {
			if (readInt(bytes, i * 4) != 0) {
				return new ChunkPos(i & 31, i >> 5);
			}
		}
		return null;
	}

	private static ChunkPos firstAbsentChunk(Path regionPath) throws IOException {
		byte[] bytes = Files.readAllBytes(regionPath);
		for (int i = 0; i < 1024; i++) {
			if (readInt(bytes, i * 4) == 0) {
				return new ChunkPos(i & 31, i >> 5);
			}
		}
		return null;
	}

	private static RawPayload readRawPayload(Path regionPath, ChunkPos pos) throws IOException {
		byte[] bytes = Files.readAllBytes(regionPath);
		int index = (pos.x & 31) + (pos.z & 31) * 32;
		int packed = readInt(bytes, index * 4);
		int timestamp = readInt(bytes, HEADER_BYTES / 2 + index * 4);
		int sector = packed >>> 8;
		int count = packed & 0xFF;
		int record = sector * SECTOR_BYTES;
		int length = readInt(bytes, record);
		int compressionByte = bytes[record + 4] & 0xFF;
		boolean external = (compressionByte & EXTERNAL_FLAG) != 0;
		int compressionId = compressionByte & ~EXTERNAL_FLAG;
		if (external) {
			byte[] payload = Files.readAllBytes(regionPath.getParent().resolve("c." + pos.x + "." + pos.z + ".mcc"));
			return new RawPayload(timestamp & 0xFFFFFFFFL, compressionId, true, payload);
		}
		int payloadLength = length - 1;
		byte[] payload = Arrays.copyOfRange(bytes, record + 5, record + 5 + payloadLength);
		assertTrue(payloadLength <= count * SECTOR_BYTES - 5);
		return new RawPayload(timestamp & 0xFFFFFFFFL, compressionId, false, payload);
	}

	private static int readInt(byte[] bytes, int offset) {
		return (bytes[offset] & 0xFF) << 24
			| (bytes[offset + 1] & 0xFF) << 16
			| (bytes[offset + 2] & 0xFF) << 8
			| bytes[offset + 3] & 0xFF;
	}

	private record RawPayload(long timestamp, int compressionId, boolean external, byte[] payload) {
	}
}
