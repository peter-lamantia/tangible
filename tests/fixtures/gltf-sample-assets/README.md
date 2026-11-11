# Khronos glTF sample fixtures

Unmodified model files from [KhronosGroup/glTF-Sample-Assets](https://github.com/KhronosGroup/glTF-Sample-Assets),
pinned to commit `8f492390ddc7a2ae221a1369ef560589cf8c70fb`.
Each model directory retains its upstream README (credits) and LICENSE.md.
Only the listed geometry variants and their buffers are included; screenshots
and other variants referenced by upstream documentation are omitted.

| Model | Included variants | Coverage |
| --- | --- | --- |
| Triangle | glTF, glTF-Embedded | Indexed triangles, external and data URI buffers |
| TriangleWithoutIndices | glTF | Non-indexed triangles |
| Box | glTF, glTF-Binary | GLB container, nested node matrix transforms |
| BoxInterleaved | glTF | Position/normal byte strides |
| SimpleSparseAccessor | glTF | Sparse position replacements |

`src/model/gltf.rs` loads these through the public model dispatcher and checks
triangle counts, bounds, and finite positions/unit normals. Tests run offline
with `cargo test --locked --all-targets`.

These fixtures test static geometry import. They do not establish support for
materials, textures, animation, skinning, or every glTF extension.

To update, retrieve the same paths under `Models/` from a reviewed upstream
commit, retain the model license and credits, and update the revision above.
