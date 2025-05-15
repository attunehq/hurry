CREATE TABLE invocation (
    invocation_id     INTEGER PRIMARY KEY,
    argv              TEXT NOT NULL,
    start_time        TEXT NOT NULL, -- RFC 3339
    end_time          TEXT           -- RFC 3339
) STRICT;

CREATE TABLE source_file (
    source_file_id    INTEGER PRIMARY KEY,
    b3sum             TEXT NOT NULL UNIQUE
) STRICT;

CREATE TABLE artifact (
    artifact_id       INTEGER PRIMARY KEY,
    b3sum             TEXT NOT NULL UNIQUE
) STRICT;

CREATE TABLE invocation_source_file (
    invocation_id     INTEGER NOT NULL,
    source_file_id    INTEGER NOT NULL,
    path              TEXT NOT NULL,
    mtime             TEXT NOT NULL, -- RFC 3339
    PRIMARY KEY (invocation_id, source_file_id, path),
    FOREIGN KEY (invocation_id) REFERENCES invocation(invocation_id),
    FOREIGN KEY (source_file_id) REFERENCES source_file(source_file_id)
) STRICT;

CREATE TABLE invocation_artifact (
    invocation_id     INTEGER NOT NULL,
    artifact_id       INTEGER NOT NULL,
    path              TEXT NOT NULL,
    mtime             TEXT NOT NULL, -- RFC 3339
    PRIMARY KEY (invocation_id, artifact_id, path),
    FOREIGN KEY (invocation_id) REFERENCES invocation(invocation_id),
    FOREIGN KEY (artifact_id) REFERENCES artifact(artifact_id)
) STRICT;
