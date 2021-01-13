use anyhow::Error;

use proxmox::tools::email::sendmail;

/// Send email to a person to request a manual media change
pub fn send_load_media_email(
    drive: &str,
    label_text: &str,
    to: &str,
) -> Result<(), Error> {

    let subject = format!("Load Media '{}' request for drive '{}'", label_text, drive);

    let mut text = String::new();

    text.push_str("Please insert the requested media into the backup drive.\n\n");

    text.push_str(&format!("Drive: {}\n", drive));
    text.push_str(&format!("Media: {}\n", label_text));

    sendmail(
        &[to],
        &subject,
        Some(&text),
        None,
        None,
        None,
    )?;

    Ok(())
}
