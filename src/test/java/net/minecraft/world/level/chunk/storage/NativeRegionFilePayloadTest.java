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
import java.io.DataOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.List;
import java.util.zip.GZIPOutputStream;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

class NativeRegionFilePayloadTest {
	private static final RegionStorageInfo STORAGE_INFO = new RegionStorageInfo("test", null, "region-test");
	private static final int SECTOR_BYTES = 4096;
	private static final int HEADER_BYTES = SECTOR_BYTES * 2;
	private static final int EXTERNAL_FLAG = 0x80;
	private static final int STATUS_CORRUPT_REGION = -3;
	private static final int STATUS_DECOMPRESSION_ERROR = -5;
	private static final int STATUS_NBT_ERROR = -6;

	@BeforeAll
	static void bootstrapMinecraft() {
		SharedConstants.tryDetectVersion();
		Bootstrap.bootStrap();
	}

	@Test
	void generatedInternalRegionsMatchJavaRegionFilePayloads(@TempDir Path tempDir) throws IOException {
		List<RegionFileVersion> versions = List.of(
			RegionFileVersion.VERSION_GZIP,
			RegionFileVersion.VERSION_DEFLATE,
			RegionFileVersion.VERSION_NONE,
			RegionFileVersion.VERSION_LZ4
		);
		ChunkPos pos = new ChunkPos(0, 0);

		for (RegionFileVersion version : versions) {
			Path dir = tempDir.resolve("format-" + version.getId());
			Files.createDirectories(dir);
			Path regionPath = dir.resolve("r.0.0.mca");
			writeJavaRegion(regionPath, dir, version, pos, sampleTag("format-" + version.getId()));

			RawPayload javaPayload = readRawPayload(regionPath, pos);
			NativeRegionFile.PayloadResult rustPayload = NativeRegionFile.readPayload(regionPath, pos.x, pos.z);

			assertEquals(NativeRegionFile.OK, rustPayload.result().status(), "format " + version.getId());
			assertTrue(rustPayload.result().present(), "format " + version.getId());
			assertEquals(version.getId(), rustPayload.result().compressionId(), "format " + version.getId());
			assertEquals(javaPayload.timestamp(), rustPayload.result().timestamp(), "format " + version.getId());
			assertFalse(rustPayload.result().external(), "format " + version.getId());
			assertArrayEquals(javaPayload.payload(), rustPayload.bytes(), "format " + version.getId());
		}
	}

	@Test
	void generatedRegionNbtFingerprintsMatchJavaForAllCompressionTypes(@TempDir Path tempDir) throws IOException {
		List<RegionFileVersion> versions = List.of(
			RegionFileVersion.VERSION_GZIP,
			RegionFileVersion.VERSION_DEFLATE,
			RegionFileVersion.VERSION_NONE,
			RegionFileVersion.VERSION_LZ4
		);
		ChunkPos pos = new ChunkPos(0, 0);

		for (RegionFileVersion version : versions) {
			Path dir = tempDir.resolve("semantic-format-" + version.getId());
			Files.createDirectories(dir);
			Path regionPath = dir.resolve("r.0.0.mca");
			writeJavaRegion(regionPath, dir, version, pos, sampleTag("semantic-" + version.getId()));

			long javaFingerprint = javaFingerprint(regionPath, pos);
			RawPayload rawPayload = readRawPayload(regionPath, pos);
			NativeRegionFile.NbtResult rust = NativeRegionFile.readNbtFingerprint(regionPath, pos.x, pos.z);

			assertEquals(NativeRegionFile.OK, rust.status(), "format " + version.getId() + " " + rust);
			assertTrue(rust.present(), "format " + version.getId());
			assertEquals(version.getId(), rust.compressionId(), "format " + version.getId());
			assertEquals(rawPayload.timestamp(), rust.timestamp(), "format " + version.getId());
			assertEquals(rawPayload.external(), rust.external(), "format " + version.getId());
			assertEquals(rawPayload.payload().length, rust.compressedLength(), "format " + version.getId());
			assertEquals(javaFingerprint, rust.fingerprint(), "format " + version.getId());
		}
	}

	@Test
	void absentChunkReportsNotPresent(@TempDir Path tempDir) throws IOException {
		Path regionPath = tempDir.resolve("r.0.0.mca");
		try (RegionFile ignored = new RegionFile(STORAGE_INFO, regionPath, tempDir, RegionFileVersion.VERSION_DEFLATE, false)) {
		}

		NativeRegionFile.PayloadResult result = NativeRegionFile.readPayload(regionPath, 0, 0);

		assertEquals(NativeRegionFile.OK, result.result().status());
		assertFalse(result.result().present());
		assertEquals(0, result.bytes().length);
	}

	@Test
	void externalMccPayloadMatchesJavaRegionFileOutput(@TempDir Path tempDir) throws IOException {
		Path regionPath = tempDir.resolve("r.0.0.mca");
		ChunkPos pos = new ChunkPos(0, 0);
		CompoundTag large = sampleTag("external");
		large.putByteArray("large", new byte[1_100_000]);

		writeJavaRegion(regionPath, tempDir, RegionFileVersion.VERSION_NONE, pos, large);

		RawPayload javaPayload = readRawPayload(regionPath, pos);
		NativeRegionFile.PayloadResult rustPayload = NativeRegionFile.readPayload(regionPath, pos.x, pos.z);

		assertTrue(javaPayload.external());
		assertEquals(NativeRegionFile.OK, rustPayload.result().status());
		assertTrue(rustPayload.result().external());
		assertEquals(RegionFileVersion.VERSION_NONE.getId(), rustPayload.result().compressionId());
		assertEquals(javaPayload.timestamp(), rustPayload.result().timestamp());
		assertArrayEquals(javaPayload.payload(), rustPayload.bytes());
	}

	@Test
	void externalMccNbtFingerprintMatchesJava(@TempDir Path tempDir) throws IOException {
		Path regionPath = tempDir.resolve("r.0.0.mca");
		ChunkPos pos = new ChunkPos(0, 0);
		CompoundTag large = sampleTag("external-semantic");
		large.putByteArray("large", new byte[1_100_000]);
		writeJavaRegion(regionPath, tempDir, RegionFileVersion.VERSION_NONE, pos, large);

		long javaFingerprint = javaFingerprint(regionPath, pos);
		RawPayload rawPayload = readRawPayload(regionPath, pos);
		NativeRegionFile.NbtResult rust = NativeRegionFile.readNbtFingerprint(regionPath, pos.x, pos.z);

		assertEquals(NativeRegionFile.OK, rust.status());
		assertTrue(rust.external());
		assertEquals(rawPayload.payload().length, rust.compressedLength());
		assertEquals(javaFingerprint, rust.fingerprint());
	}

	@Test
	void missingExternalMccFailsSafely(@TempDir Path tempDir) throws IOException {
		Path regionPath = tempDir.resolve("r.0.0.mca");
		ChunkPos pos = new ChunkPos(0, 0);
		CompoundTag large = sampleTag("external-missing");
		large.putByteArray("large", new byte[1_100_000]);
		writeJavaRegion(regionPath, tempDir, RegionFileVersion.VERSION_NONE, pos, large);
		Files.delete(tempDir.resolve("c.0.0.mcc"));

		NativeRegionFile.PayloadResult rustPayload = NativeRegionFile.readPayload(regionPath, pos.x, pos.z);

		assertEquals(-3, rustPayload.result().status());
		assertEquals(NativeRegionFile.ERROR_MISSING_EXTERNAL_FILE, rustPayload.result().errorKind());
	}

	@Test
	void corruptRegionsFailSafely(@TempDir Path tempDir) throws IOException {
		Path truncated = tempDir.resolve("truncated.mca");
		Files.write(truncated, new byte[HEADER_BYTES - 1]);
		assertEquals(STATUS_CORRUPT_REGION, NativeRegionFile.readPayload(truncated, 0, 0).result().status());

		Path invalidCompression = tempDir.resolve("invalid-compression.mca");
		byte[] region = new byte[3 * SECTOR_BYTES];
		writeInt(region, 0, (2 << 8) | 1);
		writeInt(region, HEADER_BYTES / 2, 1);
		writeInt(region, 2 * SECTOR_BYTES, 2);
		region[2 * SECTOR_BYTES + 4] = 99;
		region[2 * SECTOR_BYTES + 5] = 1;
		Files.write(invalidCompression, region);

		NativeRegionFile.PayloadResult invalidCompressionResult = NativeRegionFile.readPayload(invalidCompression, 0, 0);
		assertEquals(0, invalidCompressionResult.result().status());
		assertFalse(invalidCompressionResult.result().present());
	}

	@Test
	void semanticOperationReportsDecompressionAndNbtErrors(@TempDir Path tempDir) throws IOException {
		Path badCompressed = tempDir.resolve("bad-compressed.mca");
		writeManualRegion(badCompressed, RegionFileVersion.VERSION_DEFLATE.getId(), new byte[]{1, 2, 3, 4});

		NativeRegionFile.NbtResult decompression = NativeRegionFile.readNbtFingerprint(badCompressed, 0, 0);
		assertEquals(STATUS_DECOMPRESSION_ERROR, decompression.status());
		assertEquals(2, decompression.errorDomain());

		Path badNbt = tempDir.resolve("bad-nbt.mca");
		writeManualRegion(badNbt, RegionFileVersion.VERSION_GZIP.getId(), gzip(new byte[]{10, 0}));

		NativeRegionFile.NbtResult nbt = NativeRegionFile.readNbtFingerprint(badNbt, 0, 0);
		assertEquals(STATUS_NBT_ERROR, nbt.status());
		assertEquals(3, nbt.errorDomain());

		Path limited = tempDir.resolve("limited.mca");
		ChunkPos pos = new ChunkPos(0, 0);
		writeJavaRegion(limited, tempDir, RegionFileVersion.VERSION_DEFLATE, pos, sampleTag("limited"));
		NativeRegionFile.NbtResult sizeLimit = NativeRegionFile.readNbtFingerprint(limited, 0, 0, 0, 3, 0, 0, 0, 0);
		assertEquals(STATUS_DECOMPRESSION_ERROR, sizeLimit.status());
		assertEquals(2, sizeLimit.errorDomain());

		Path goodLz4 = tempDir.resolve("good-lz4.mca");
		Path badLz4 = tempDir.resolve("bad-lz4.mca");
		writeJavaRegion(goodLz4, tempDir, RegionFileVersion.VERSION_LZ4, pos, sampleTag("bad-lz4"));
		byte[] corruptLz4 = readRawPayload(goodLz4, pos).payload().clone();
		corruptLz4[21] ^= 0x55;
		writeManualRegion(badLz4, RegionFileVersion.VERSION_LZ4.getId(), corruptLz4);

		NativeRegionFile.NbtResult lz4 = NativeRegionFile.readNbtFingerprint(badLz4, 0, 0);
		assertEquals(STATUS_DECOMPRESSION_ERROR, lz4.status());
		assertEquals(2, lz4.errorDomain());
		assertThrows(IOException.class, () -> javaFingerprint(badLz4, pos));
	}

	@Test
	void copiedRealWorldRegionPayloadsMatchRust(@TempDir Path tempDir) throws IOException {
		Path source = Path.of("run", "saves", "mattmc-real-meshing-replay", "region", "r.0.0.mca");
		assumeTrue(Files.isRegularFile(source), "real-world meshing replay region is not available");
		Path copy = tempDir.resolve("r.0.0.mca");
		Files.copy(source, copy);

		ChunkPos pos = firstPresentChunk(copy);
		assertNotNull(pos, "real region must contain at least one chunk");
		RawPayload javaPayload = readRawPayload(copy, pos);
		NativeRegionFile.PayloadResult rustPayload = NativeRegionFile.readPayload(copy, pos.x, pos.z);

		assertEquals(NativeRegionFile.OK, rustPayload.result().status());
		assertTrue(rustPayload.result().present());
		assertEquals(javaPayload.compressionId(), rustPayload.result().compressionId());
		assertEquals(javaPayload.timestamp(), rustPayload.result().timestamp());
		assertEquals(javaPayload.external(), rustPayload.result().external());
		assertArrayEquals(javaPayload.payload(), rustPayload.bytes());
	}

	@Test
	void copiedRealWorldRegionNbtFingerprintMatchesJava(@TempDir Path tempDir) throws IOException {
		Path source = Path.of("run", "saves", "mattmc-real-meshing-replay", "region", "r.0.0.mca");
		assumeTrue(Files.isRegularFile(source), "real-world meshing replay region is not available");
		Path copy = tempDir.resolve("r.0.0.mca");
		Files.copy(source, copy);

		ChunkPos pos = firstPresentChunk(copy);
		assertNotNull(pos, "real region must contain at least one chunk");
		long javaFingerprint = javaFingerprint(copy, pos);
		NativeRegionFile.NbtResult rust = NativeRegionFile.readNbtFingerprint(copy, pos.x, pos.z);

		assertEquals(NativeRegionFile.OK, rust.status());
		assertTrue(rust.present());
		assertEquals(javaFingerprint, rust.fingerprint());
		assertTrue(rust.compressedLength() > 0);
		assertTrue(rust.decompressedLength() > 0);
	}

	private static void writeJavaRegion(Path regionPath, Path dir, RegionFileVersion version, ChunkPos pos, CompoundTag tag) throws IOException {
		try (RegionFile regionFile = new RegionFile(STORAGE_INFO, regionPath, dir, version, false);
			DataOutputStream output = regionFile.getChunkDataOutputStream(pos)) {
			NbtIo.write(tag, output);
		}
	}

	private static CompoundTag sampleTag(String name) {
		CompoundTag tag = new CompoundTag();
		tag.putString("name", name);
		tag.putInt("x", 123);
		tag.putLong("seed", 987654321L);
		tag.putByteArray("bytes", new byte[]{1, 2, 3, 4});
		return tag;
	}

	private static long javaFingerprint(Path regionPath, ChunkPos pos) throws IOException {
		try (RegionFile regionFile = new RegionFile(STORAGE_INFO, regionPath, regionPath.getParent(), false);
			InputStream input = regionFile.getChunkDataInputStream(pos)) {
			assertNotNull(input);
			CompoundTag tag = NbtIo.read(new java.io.DataInputStream(input), NbtAccounter.unlimitedHeap());
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
				int localX = i & 31;
				int localZ = i >> 5;
				return new ChunkPos(localX, localZ);
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

	private static void writeInt(byte[] bytes, int offset, int value) {
		bytes[offset] = (byte)(value >>> 24);
		bytes[offset + 1] = (byte)(value >>> 16);
		bytes[offset + 2] = (byte)(value >>> 8);
		bytes[offset + 3] = (byte)value;
	}

	private static void writeManualRegion(Path regionPath, int compressionId, byte[] payload) throws IOException {
		byte[] region = new byte[3 * SECTOR_BYTES];
		writeInt(region, 0, (2 << 8) | 1);
		writeInt(region, HEADER_BYTES / 2, 1);
		writeInt(region, 2 * SECTOR_BYTES, payload.length + 1);
		region[2 * SECTOR_BYTES + 4] = (byte)compressionId;
		System.arraycopy(payload, 0, region, 2 * SECTOR_BYTES + 5, payload.length);
		Files.write(regionPath, region);
	}

	private static byte[] gzip(byte[] input) throws IOException {
		ByteArrayOutputStream bytes = new ByteArrayOutputStream();
		try (GZIPOutputStream gzip = new GZIPOutputStream(bytes)) {
			gzip.write(input);
		}
		return bytes.toByteArray();
	}

	private record RawPayload(long timestamp, int compressionId, boolean external, byte[] payload) {
	}
}
