/*!
    Opaque codec configuration for passing to decoders.
*/

use ffmpeg_next::codec;

/**
    Opaque codec configuration extracted from a source stream.

    This holds the codec parameters needed to create a decoder.
    It's intentionally opaque to hide ffmpeg-next types from the public API.

    Pass this to `ffmpeg-decode` to create a decoder for this stream.
*/
pub struct CodecConfig {
    /// The raw codec parameters.
    pub(crate) parameters: codec::Parameters,
}

impl CodecConfig {
    /**
        Create a new codec config from ffmpeg parameters.
    */
    pub(crate) fn new(parameters: codec::Parameters) -> Self {
        Self { parameters }
    }

    /**
        Get a reference to the internal parameters.

        This is pub(crate) to allow ffmpeg-decode to access it.
        We'll need to make this accessible across crate boundaries later,
        possibly via a shared internal crate or unsafe accessor.
    */
    pub fn into_parameters(self) -> codec::Parameters {
        self.parameters
    }
}

impl Clone for CodecConfig {
    fn clone(&self) -> Self {
        Self {
            parameters: self.parameters.clone(),
        }
    }
}

impl std::fmt::Debug for CodecConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodecConfig")
            .field("codec_id", &self.parameters.id())
            .finish_non_exhaustive()
    }
}
