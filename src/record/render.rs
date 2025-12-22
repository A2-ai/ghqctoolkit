use std::{
    collections::{BTreeMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    io::{self, BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
};

use libreofficekit::{DocUrl, Office};
use lopdf::{Bookmark, Document, Object, ObjectId};

/// Trait for converting documents (Word, etc.) to PDF
pub trait DocumentConverter {
    /// Convert a document file to PDF and return the loaded PDF Document
    fn convert_to_pdf(&self, input: &Path, output_dir: &Path) -> Result<Document, RenderError>;
}

/// LibreOffice-based document converter
pub struct LibreOfficeConverter {
    office: Office,
}

impl LibreOfficeConverter {
    /// Try to create a converter, returns None if LibreOffice isn't installed
    pub fn try_new() -> Option<Self> {
        Office::find_install_path()
            .and_then(|p| {
                log::debug!("Found LibreOffice at: {}", p.display());
                Office::new(p).ok()
            })
            .map(|office| Self { office })
    }
}

impl DocumentConverter for LibreOfficeConverter {
    fn convert_to_pdf(&self, input: &Path, output_dir: &Path) -> Result<Document, RenderError> {
        let output_file_name = input.with_extension("pdf");
        let output_file = output_dir.join(output_file_name.file_name().ok_or(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "provided context file does not have file name: {}",
                input.display()
            ),
        ))?);

        log::debug!(
            "Converting {} to PDF at {}",
            input.display(),
            output_file.display()
        );

        let input_doc = DocUrl::from_relative_path(input.to_string_lossy())?;
        let output_pdf = DocUrl::from_absolute_path(output_file.to_string_lossy())?;

        let mut document = self.office.document_load(&input_doc)?;
        let success = document.save_as(&output_pdf, "pdf", None)?;

        if !success {
            log::error!("Failed to convert {} to PDF", input.display());
            return Err(RenderError::OfficeError(
                libreofficekit::OfficeError::OfficeError(format!(
                    "Conversion of {} to PDF was not successful",
                    input.display()
                )),
            ));
        }

        Document::load(&output_file).map_err(|error| RenderError::PdfReadError {
            file: output_file,
            error,
        })
    }
}

#[derive(Debug, Clone)]
pub struct QCContext {
    file: PathBuf,
    position: ContextPosition,
}

impl QCContext {
    pub fn new(path: impl AsRef<Path>, position: ContextPosition) -> Self {
        Self {
            file: path.as_ref().to_path_buf(),
            position,
        }
    }

    pub fn file(&self) -> &Path {
        &self.file
    }

    pub fn position(&self) -> ContextPosition {
        self.position
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ContextPosition {
    Prepend,
    Append,
}

/// Render a Quarto document to PDF using the quarto CLI tool
///
/// # Arguments
/// * `report` - The Quarto markdown content to render
/// * `path` - The output path for the rendered PDF (without extension)
///
/// # Returns
/// * `Ok(())` - If rendering succeeded
/// * `Err(RecordError)` - If rendering failed
///
/// # Example
/// ```no_run
/// use std::path::Path;
/// use ghqctoolkit::render;
///
/// let report = "---\ntitle: My Report\n---\n# Hello World";
/// render(report, Path::new("output/my-report")).unwrap();
/// // Creates output/my-report.pdf
/// ```
pub fn render(
    record_str: &str,
    path: impl AsRef<Path>,
    qc_context: &[QCContext],
) -> Result<(), RenderError> {
    let output_path = path.as_ref();

    // Create staging directory using hash of report content
    let mut hasher = DefaultHasher::new();
    record_str.hash(&mut hasher);
    let hash = hasher.finish();
    let staging_dir = std::env::temp_dir().join(format!("ghqc-render-{:x}", hash));
    std::fs::create_dir_all(&staging_dir)?;

    let cleanup_staging = || {
        if let Err(e) = std::fs::remove_dir_all(&staging_dir) {
            log::warn!(
                "Failed to cleanup staging directory {}: {}",
                staging_dir.display(),
                e
            );
        }
    };

    let findings_doc = render_findings_in_staging(&staging_dir, record_str).and_then(|file| {
        Document::load(&file).map_err(|error| RenderError::PdfReadError { file, error })
    })?;
    let mut doc = if !qc_context.is_empty() {
        let converter = LibreOfficeConverter::try_new();
        let context_docs = qc_context
            .iter()
            .map(|context| {
                load_context_file(&context.file, &staging_dir, converter.as_ref())
                    .map(|d| (d, context.position))
            })
            .collect::<Result<Vec<(Document, ContextPosition)>, RenderError>>()?;

        merge_pdfs(findings_doc, context_docs)?
    } else {
        findings_doc
    };

    doc.save(output_path)?;

    // Always cleanup staging directory
    cleanup_staging();

    Ok(())
}

fn merge_pdfs(
    findings_doc: Document,
    context_docs: Vec<(Document, ContextPosition)>,
) -> Result<Document, RenderError> {
    let mut prepended_docs = Vec::new();
    let mut appended_docs = Vec::new();
    for (doc, pos) in context_docs {
        match pos {
            ContextPosition::Prepend => prepended_docs.push(doc),
            ContextPosition::Append => appended_docs.push(doc),
        }
    }
    let mut docs = prepended_docs;
    docs.push(findings_doc);
    docs.append(&mut appended_docs);

    // Define a starting `max_id` (will be used as start index for object_ids).
    let mut max_id = 1;
    let mut pagenum = 1;
    // Collect all Documents Objects grouped by a map
    let mut documents_pages = BTreeMap::new();
    let mut documents_objects = BTreeMap::new();
    let mut document = Document::with_version("1.5");

    for mut doc in docs {
        let mut first = false;
        doc.renumber_objects_with(max_id);

        max_id = doc.max_id + 1;

        for (_, object_id) in doc.get_pages() {
            if !first {
                let bookmark = Bookmark::new(
                    String::from(format!("Page_{}", pagenum)),
                    [0.0, 0.0, 1.0],
                    0,
                    object_id,
                );
                document.add_bookmark(bookmark, None);
                first = true;
                pagenum += 1;
            }

            let obj = doc
                .get_object(object_id)
                .map_err(|_| RenderError::PdfMergeMissingObject(object_id))?
                .to_owned();
            documents_pages.insert(object_id, obj);
        }
        documents_objects.extend(doc.objects);
    }

    // "Catalog" and "Pages" are mandatory.
    let mut catalog_object: Option<(ObjectId, Object)> = None;
    let mut pages_object: Option<(ObjectId, Object)> = None;

    // Process all objects except "Page" type
    for (object_id, object) in documents_objects.iter() {
        // We have to ignore "Page" (as are processed later), "Outlines" and "Outline" objects.
        // All other objects should be collected and inserted into the main Document.
        match object.type_name().unwrap_or(b"") {
            b"Catalog" => {
                // Collect a first "Catalog" object and use it for the future "Pages".
                catalog_object = Some((
                    if let Some((id, _)) = catalog_object {
                        id
                    } else {
                        *object_id
                    },
                    object.clone(),
                ));
            }
            b"Pages" => {
                // Collect and update a first "Pages" object and use it for the future "Catalog"
                // We have also to merge all dictionaries of the old and the new "Pages" object
                if let Ok(dictionary) = object.as_dict() {
                    let mut dictionary = dictionary.clone();
                    if let Some((_, ref object)) = pages_object {
                        if let Ok(old_dictionary) = object.as_dict() {
                            dictionary.extend(old_dictionary);
                        }
                    }

                    pages_object = Some((
                        if let Some((id, _)) = pages_object {
                            id
                        } else {
                            *object_id
                        },
                        Object::Dictionary(dictionary),
                    ));
                }
            }
            b"Page" => {}     // Ignored, processed later and separately
            b"Outlines" => {} // Ignored, not supported yet
            b"Outline" => {}  // Ignored, not supported yet
            _ => {
                document.objects.insert(*object_id, object.clone());
            }
        }
    }

    // If no "Pages" object found, abort.
    let Some(pages_object) = pages_object else {
        return Err(RenderError::PdfMergeMissingPages);
    };

    // Iterate over all "Page" objects and collect into the parent "Pages" created before
    for (object_id, object) in documents_pages.iter() {
        if let Ok(dictionary) = object.as_dict() {
            let mut dictionary = dictionary.clone();
            dictionary.set("Parent", pages_object.0);

            document
                .objects
                .insert(*object_id, Object::Dictionary(dictionary));
        }
    }

    // If no "Catalog" found, abort.
    let Some(catalog_object) = catalog_object else {
        return Err(RenderError::PdfMergeMissingCatalog);
    };

    // Build a new "Pages" with updated fields
    if let Ok(dictionary) = pages_object.1.as_dict() {
        let mut dictionary = dictionary.clone();

        // Set new pages count
        dictionary.set("Count", documents_pages.len() as u32);

        // Set new "Kids" list (collected from documents pages) for "Pages"
        dictionary.set(
            "Kids",
            documents_pages
                .into_iter()
                .map(|(object_id, _)| Object::Reference(object_id))
                .collect::<Vec<_>>(),
        );

        document
            .objects
            .insert(pages_object.0, Object::Dictionary(dictionary));
    }

    // Build a new "Catalog" with updated fields
    if let Ok(dictionary) = catalog_object.1.as_dict() {
        let mut dictionary = dictionary.clone();
        dictionary.set("Pages", pages_object.0);
        dictionary.remove(b"Outlines"); // Outlines not supported in merged PDFs

        document
            .objects
            .insert(catalog_object.0, Object::Dictionary(dictionary));
    }

    document.trailer.set("Root", catalog_object.0);

    // Update the max internal ID as wasn't updated before due to direct objects insertion
    document.max_id = document.objects.len() as u32;

    // Reorder all new Document objects
    document.renumber_objects();

    // Set any Bookmarks to the First child if they are not set to a page
    document.adjust_zero_pages();

    // Set all bookmarks to the PDF Object tree then set the Outlines to the Bookmark content map.
    if let Some(n) = document.build_outline() {
        if let Ok(Object::Dictionary(dict)) = document.get_object_mut(catalog_object.0) {
            dict.set("Outlines", Object::Reference(n));
        }
    }

    document.compress();

    Ok(document)
}

fn render_findings_in_staging(
    staging_dir: impl AsRef<Path>,
    report: &str,
) -> Result<PathBuf, RenderError> {
    let staging_dir = staging_dir.as_ref();
    let qmd_file = staging_dir.join("record.qmd");
    let staging_pdf_path = staging_dir.join("record.pdf");

    log::debug!("Writing Quarto document to staging: {}", qmd_file.display());
    std::fs::write(&qmd_file, report)?;

    log::debug!("Rendering findings PDF with Quarto: {}", qmd_file.display());

    // Execute quarto render command with combined stdout/stderr
    let mut cmd = Command::new("quarto");
    cmd.args(&["render", qmd_file.to_str().unwrap()])
        .current_dir(staging_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped()); // Capture stderr separately

    log::debug!("Executing command: {:?}", cmd);

    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            RenderError::QuartoNotFound
        } else {
            RenderError::Io(e)
        }
    })?;

    // Collect both stdout and stderr
    let stdout = child.stdout.take().expect("Failed to get stdout");
    let stderr = child.stderr.take().expect("Failed to get stderr");

    let stdout_handle = thread::spawn(move || {
        let mut lines = Vec::new();
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            if let Ok(line) = line {
                lines.push(line);
            }
        }
        lines
    });

    let stderr_handle = thread::spawn(move || {
        let mut lines = Vec::new();
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            if let Ok(line) = line {
                lines.push(line);
            }
        }
        lines
    });

    // Wait for process to complete
    let exit_status = child.wait()?;

    // Get the collected output from both streams
    let stdout_lines = stdout_handle
        .join()
        .unwrap_or_else(|_| vec!["Failed to collect stdout".to_string()]);
    let stderr_lines = stderr_handle
        .join()
        .unwrap_or_else(|_| vec!["Failed to collect stderr".to_string()]);

    let mut combined_lines = Vec::new();
    combined_lines.extend(stdout_lines);
    combined_lines.extend(stderr_lines);
    let combined_output = combined_lines.join("\n");

    // Check if command succeeded
    if !exit_status.success() {
        let exit_code = exit_status.code().unwrap_or(-1);
        return Err(RenderError::QuartoRenderFailed {
            code: exit_code,
            stderr: combined_output,
        });
    }

    // Verify PDF was created in staging
    if !staging_pdf_path.exists() {
        return Err(RenderError::Io(io::Error::new(
            io::ErrorKind::NotFound,
            format!("PDF not created in staging: {}", staging_pdf_path.display()),
        )));
    }

    log::debug!("Successfully rendered PDF: {}", staging_pdf_path.display());

    Ok(staging_pdf_path)
}

/// Load a context file as a PDF Document.
/// PDFs are loaded directly; other formats require a converter.
fn load_context_file(
    file: impl AsRef<Path>,
    staging_dir: impl AsRef<Path>,
    converter: Option<&impl DocumentConverter>,
) -> Result<Document, RenderError> {
    let input_file = file.as_ref();

    // Check if the file is already a PDF - load it directly
    let is_pdf = input_file
        .extension()
        .map(|ext| ext.to_string_lossy().to_lowercase() == "pdf")
        .unwrap_or(false);

    if is_pdf {
        log::debug!("Loading PDF directly: {}", input_file.display());
        return Document::load(input_file).map_err(|error| RenderError::PdfReadError {
            file: input_file.to_path_buf(),
            error,
        });
    }

    // For non-PDF files, we need a converter
    let Some(converter) = converter else {
        return Err(RenderError::ConverterUnavailable {
            file: input_file.to_path_buf(),
        });
    };

    converter.convert_to_pdf(input_file, staging_dir.as_ref())
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("Quarto render failed with exit code {code}: {stderr}")]
    QuartoRenderFailed { code: i32, stderr: String },
    #[error(
        "Quarto command not found. Please install Quarto: https://quarto.org/docs/get-started/"
    )]
    QuartoNotFound,
    #[error("IO Error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Failed to convert office file to pdf: {0}")]
    OfficeError(#[from] libreofficekit::OfficeError),
    #[error("Failed to read pdf at {file}: {error}")]
    PdfReadError { file: PathBuf, error: lopdf::Error },
    #[error("Failed to write pdf: {0}")]
    PdfWriteError(#[from] lopdf::Error),
    #[error("PDF merge failed: Pages root not found in source documents")]
    PdfMergeMissingPages,
    #[error("PDF merge failed: Catalog not found in source documents")]
    PdfMergeMissingCatalog,
    #[error("PDF merge failed: Could not retrieve object {0:?}")]
    PdfMergeMissingObject(ObjectId),
    #[error(
        "Cannot convert {file} to PDF: LibreOffice is not available. Install LibreOffice or use a PDF file directly."
    )]
    ConverterUnavailable { file: PathBuf },
}
