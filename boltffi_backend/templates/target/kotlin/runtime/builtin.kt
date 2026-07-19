{% if split_data_package %}internal{% else %}private{% endif %} fun WireReader.readDuration(): java.time.Duration {
    val seconds = readI64()
    val nanos = readI32().toLong()
    require(seconds >= 0L) { "Duration out of range" }
    require(nanos >= 0L) { "Duration nanos out of range" }
    return java.time.Duration.ofSeconds(seconds, nanos)
}

{% if split_data_package %}internal{% else %}private{% endif %} fun WireReader.readInstant(): java.time.Instant {
    val seconds = readI64()
    val nanos = readI32()
    require(nanos >= 0L) { "Instant nanos out of range" }
    return java.time.Instant.ofEpochSecond(seconds, nanos.toLong())
}

{% if split_data_package %}internal{% else %}private{% endif %} fun WireReader.readUuid(): java.util.UUID = java.util.UUID(readI64(), readI64())

{% if split_data_package %}internal{% else %}private{% endif %} fun WireReader.readUri(): java.net.URI = java.net.URI.create(readString())

{% if split_data_package %}internal{% else %}private{% endif %} fun WireWriter.writeDuration(value: java.time.Duration) {
    require(value.seconds >= 0L) { "Invalid duration, must be non-negative" }
    require(value.nano >= 0) { "Invalid duration nanos" }
    writeI64(value.seconds)
    writeI32(value.nano)
}

{% if split_data_package %}internal{% else %}private{% endif %} fun WireWriter.writeInstant(value: java.time.Instant) {
    writeI64(value.epochSecond)
    writeI32(value.nano)
}

{% if split_data_package %}internal{% else %}private{% endif %} fun WireWriter.writeUuid(value: java.util.UUID) {
    writeI64(value.mostSignificantBits)
    writeI64(value.leastSignificantBits)
}

{% if split_data_package %}internal{% else %}private{% endif %} fun WireWriter.writeUri(value: java.net.URI) {
    writeString(value.toString())
}
