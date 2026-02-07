//! End-to-end PlayReady license acquisition against Microsoft's public test server.
//!
//! Uses the "Tears of Steel 4K" test content with a hardcoded PSSH and a random
//! embedded SL3000 device.
//!
//! Run with:
//!     cargo run -p drm-playready --example get_keys --features static-devices

use drm_playready::PlayReadyExt;

const PSSH_B64: &str = "\
    AAADfHBzc2gAAAAAmgTweZhAQoarkuZb4IhflQAAA1xcAwAAAQABAFIDPABXAFIATQBIAEUAQQBEAEUAUgAgAHgAbQBsAG4AcwA9ACIAaAB0AH\
    QAcAA6AC8ALwBzAGMAaABlAG0AYQBzAC4AbQBpAGMAcgBvAHMAbwBmAHQALgBjAG8AbQAvAEQAUgBNAC8AMgAwADAANwAvADAAMwAvAFAAbABh\
    AHkAUgBlAGEAZAB5AEgAZQBhAGQAZQByACIAIAB2AGUAcgBzAGkAbwBuAD0AIgA0AC4AMAAuADAALgAwACIAPgA8AEQAQQBUAEEAPgA8AFAAUg\
    BPAFQARQBDAFQASQBOAEYATwA+ADwASwBFAFkATABFAE4APgAxADYAPAAvAEsARQBZAEwARQBOAD4APABBAEwARwBJAEQAPgBBAEUAUwBDAFQA\
    UgA8AC8AQQBMAEcASQBEAD4APAAvAFAAUgBPAFQARQBDAFQASQBOAEYATwA+ADwASwBJAEQAPgA0AFIAcABsAGIAKwBUAGIATgBFAFMAOAB0AE\
    cAawBOAEYAVwBUAEUASABBAD0APQA8AC8ASwBJAEQAPgA8AEMASABFAEMASwBTAFUATQA+AEsATABqADMAUQB6AFEAUAAvAE4AQQA9ADwALwBD\
    AEgARQBDAEsAUwBVAE0APgA8AEwAQQBfAFUAUgBMAD4AaAB0AHQAcABzADoALwAvAHAAcgBvAGYAZgBpAGMAaQBhAGwAcwBpAHQAZQAuAGsAZQ\
    B5AGQAZQBsAGkAdgBlAHIAeQAuAG0AZQBkAGkAYQBzAGUAcgB2AGkAYwBlAHMALgB3AGkAbgBkAG8AdwBzAC4AbgBlAHQALwBQAGwAYQB5AFIA\
    ZQBhAGQAeQAvADwALwBMAEEAXwBVAFIATAA+ADwAQwBVAFMAVABPAE0AQQBUAFQAUgBJAEIAVQBUAEUAUwA+ADwASQBJAFMAXwBEAFIATQBfAF\
    YARQBSAFMASQBPAE4APgA4AC4AMQAuADIAMwAwADQALgAzADEAPAAvAEkASQBTAF8ARABSAE0AXwBWAEUAUgBTAEkATwBOAD4APAAvAEMAVQBT\
    AFQATwBNAEEAVABUAFIASQBCAFUAVABFAFMAPgA8AC8ARABBAFQAQQA+ADwALwBXAFIATQBIAEUAQQBEAEUAUgA+AA==";

const LICENSE_SERVER: &str = "https://test.playready.microsoft.com/service/rightsmanager.asmx?cfg=(persist:false,sl:3000,ckt:aesctr)";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Pick a random embedded SL3000 device
    let device = drm_playready::static_devices::random();
    eprintln!("Device: {}", device.security_level);

    // Open a session
    let mut session = drm_playready::Session::new(device);

    // Parse the Tears of Steel PSSH
    let pssh = drm_core::PsshBox::from_base64(PSSH_B64)?;

    let key_ids = pssh.playready_key_ids()?;
    eprintln!("PSSH contains {} key ID(s):", key_ids.len());
    for kid in &key_ids {
        eprintln!("  {}", hex::encode(kid));
    }

    // Build the SOAP license challenge
    let challenge = session.build_license_challenge(&pssh)?;
    eprintln!("Built challenge ({} bytes)", challenge.len());

    // POST to Microsoft's public test server
    eprintln!("Sending to {LICENSE_SERVER}");
    let response = reqwest::Client::new()
        .post(LICENSE_SERVER)
        .header("Content-Type", "text/xml; charset=utf-8")
        .body(challenge)
        .send()
        .await?;

    let status = response.status();
    let body = response.bytes().await?;

    if !status.is_success() {
        anyhow::bail!(
            "license server returned HTTP {status}: {}",
            String::from_utf8_lossy(&body)
        );
    }

    // Parse the license response and extract content keys
    let keys = session.parse_license_response(&body)?;

    eprintln!("Got {} key(s):", keys.len());
    for key in keys {
        println!("{key}");
    }

    // Print XMR license metadata
    for (i, license) in session.licenses().iter().enumerate() {
        eprintln!("License {i} metadata:");
        if let Some(exp) = license.find_expiration() {
            eprintln!(
                "  Expiration: begin={} end={}",
                exp.begin_date, exp.end_date
            );
        }
        if let Some(sl) = license.find_security_level() {
            eprintln!("  Security level: {}", sl.minimum_security_level);
        }
        if let Some(issue) = license.find_issue_date() {
            eprintln!("  Issue date: {}", issue.issue_date);
        }
        if let Some(op) = license.find_output_protection() {
            eprintln!(
                "  Output protection: compressed={} uncompressed={} analog={}",
                op.compressed_digital_video, op.uncompressed_digital_video, op.analog_video
            );
        }
    }

    Ok(())
}
