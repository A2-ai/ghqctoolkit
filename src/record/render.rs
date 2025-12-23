use std::{
    collections::{BTreeMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    io::{self, BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use libreofficekit::{DocUrl, Office};
use lopdf::{Bookmark, Document, Object, ObjectId};

/// Create a staging directory for record generation
///
/// Creates a unique staging directory in the system temp folder using a hash.
/// This directory is used to store downloaded images, the logo, and the rendered template.
pub fn create_staging_dir() -> Result<PathBuf, RenderError> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    let mut hasher = DefaultHasher::new();
    timestamp.hash(&mut hasher);
    let hash = hasher.finish();

    let staging_dir = std::env::temp_dir().join(format!("ghqc-render-{:x}", hash));
    std::fs::create_dir_all(&staging_dir)?;

    log::debug!("Created staging directory: {}", staging_dir.display());
    Ok(staging_dir)
}

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

/// Render a Typst document to PDF using the typst CLI tool
///
/// # Arguments
/// * `record_str` - The Typst document content to render
/// * `path` - The output path for the rendered PDF
/// * `staging_dir` - The staging directory containing the template and assets (images, logo)
/// * `qc_context` - Optional context files to prepend/append to the PDF
///
/// # Returns
/// * `Ok(())` - If rendering succeeded
/// * `Err(RenderError)` - If rendering failed
///
/// # Example
/// ```no_run
/// use std::path::Path;
/// use ghqctoolkit::{render, QCContext, ContextPosition, create_staging_dir};
///
/// let report = "#set document(title: \"My Report\")\n= Hello World";
/// let staging_dir = create_staging_dir().unwrap();
/// render(report, Path::new("output/my-report.pdf"), &staging_dir, &[]).unwrap();
/// ```
pub fn render(
    record_str: &str,
    path: impl AsRef<Path>,
    staging_dir: impl AsRef<Path>,
    qc_context: &[QCContext],
) -> Result<(), RenderError> {
    let output_path = path.as_ref();
    let staging_dir = staging_dir.as_ref();

    let cleanup_staging = || {
        if let Err(e) = std::fs::remove_dir_all(staging_dir) {
            log::warn!(
                "Failed to cleanup staging directory {}: {}",
                staging_dir.display(),
                e
            );
        }
    };

    let findings_doc = render_typst_in_staging(staging_dir, record_str).and_then(|file| {
        Document::load(&file).map_err(|error| RenderError::PdfReadError { file, error })
    })?;

    let mut doc = if !qc_context.is_empty() {
        let converter = LibreOfficeConverter::try_new();
        let context_docs = qc_context
            .iter()
            .map(|context| {
                load_context_file(&context.file, staging_dir, converter.as_ref())
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

fn render_typst_in_staging(staging_dir: &Path, report: &str) -> Result<PathBuf, RenderError> {
    let typ_file = staging_dir.join("record.typ");
    let staging_pdf_path = staging_dir.join("record.pdf");

    log::debug!("Writing Typst document to staging: {}", typ_file.display());
    std::fs::write(&typ_file, report)?;

    log::debug!(
        "Rendering PDF with Typst: {} -> {}",
        typ_file.display(),
        staging_pdf_path.display()
    );

    // Execute typst compile command with combined stdout/stderr
    let mut cmd = Command::new("typst");
    cmd.args([
        "compile",
        typ_file.to_str().unwrap(),
        staging_pdf_path.to_str().unwrap(),
    ])
    .current_dir(staging_dir)
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());

    log::debug!("Executing command: {:?}", cmd);

    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            RenderError::TypstNotFound
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
        return Err(RenderError::TypstRenderFailed {
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
    #[error("Typst render failed with exit code {code}: {stderr}")]
    TypstRenderFailed { code: i32, stderr: String },
    #[error("Typst command not found. Please install Typst: https://typst.app")]
    TypstNotFound,
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // Helper to create a minimal valid PDF for testing
    fn create_test_pdf() -> Document {
        use lopdf::dictionary;

        let mut doc = Document::with_version("1.5");

        // Create a minimal page
        let pages_id = doc.new_object_id();
        let page_id = doc.new_object_id();
        let content_id = doc.new_object_id();

        // Add content stream (empty)
        doc.objects.insert(
            content_id,
            Object::Stream(lopdf::Stream::new(dictionary! {}, vec![])),
        );

        // Add page
        let page = dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        };
        doc.objects.insert(page_id, Object::Dictionary(page));

        // Add pages
        let pages = dictionary! {
            "Type" => "Pages",
            "Kids" => vec![Object::Reference(page_id)],
            "Count" => 1,
        };
        doc.objects.insert(pages_id, Object::Dictionary(pages));

        // Add catalog
        let catalog_id = doc.new_object_id();
        let catalog = dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        };
        doc.objects.insert(catalog_id, Object::Dictionary(catalog));

        // Set trailer
        doc.trailer.set("Root", catalog_id);
        doc.max_id = doc.objects.len() as u32;

        doc
    }

    // Mock converter for testing
    struct MockConverter {
        should_succeed: bool,
    }

    impl DocumentConverter for MockConverter {
        fn convert_to_pdf(
            &self,
            input: &Path,
            _output_dir: &Path,
        ) -> Result<Document, RenderError> {
            if self.should_succeed {
                Ok(create_test_pdf())
            } else {
                Err(RenderError::OfficeError(
                    libreofficekit::OfficeError::OfficeError(format!(
                        "Mock conversion failed for {}",
                        input.display()
                    )),
                ))
            }
        }
    }

    // ===================
    // QCContext tests
    // ===================

    #[test]
    fn test_qc_context_new() {
        let ctx = QCContext::new("/path/to/file.pdf", ContextPosition::Prepend);
        assert_eq!(ctx.file(), Path::new("/path/to/file.pdf"));
        assert!(matches!(ctx.position(), ContextPosition::Prepend));
    }

    #[test]
    fn test_qc_context_append_position() {
        let ctx = QCContext::new("document.docx", ContextPosition::Append);
        assert_eq!(ctx.file(), Path::new("document.docx"));
        assert!(matches!(ctx.position(), ContextPosition::Append));
    }

    #[test]
    fn test_qc_context_clone() {
        let ctx1 = QCContext::new("/test/path.pdf", ContextPosition::Prepend);
        let ctx2 = ctx1.clone();
        assert_eq!(ctx1.file(), ctx2.file());
        assert!(matches!(ctx1.position(), ContextPosition::Prepend));
        assert!(matches!(ctx2.position(), ContextPosition::Prepend));
    }

    // ===================
    // load_context_file tests
    // ===================

    #[test]
    fn test_load_context_file_pdf_directly() {
        let temp_dir = TempDir::new().unwrap();
        let pdf_path = temp_dir.path().join("test.pdf");

        // Create and save a test PDF
        let mut doc = create_test_pdf();
        doc.save(&pdf_path).unwrap();

        // Load without converter (should work for PDFs)
        let result = load_context_file(&pdf_path, temp_dir.path(), None::<&MockConverter>);
        assert!(result.is_ok());
    }

    #[test]
    fn test_load_context_file_non_pdf_without_converter() {
        let temp_dir = TempDir::new().unwrap();
        let docx_path = temp_dir.path().join("test.docx");

        // Create a dummy file
        std::fs::write(&docx_path, b"dummy content").unwrap();

        // Try to load without converter
        let result = load_context_file(&docx_path, temp_dir.path(), None::<&MockConverter>);

        assert!(result.is_err());
        match result.unwrap_err() {
            RenderError::ConverterUnavailable { file } => {
                assert_eq!(file, docx_path);
            }
            e => panic!("Expected ConverterUnavailable, got {:?}", e),
        }
    }

    #[test]
    fn test_load_context_file_non_pdf_with_converter() {
        let temp_dir = TempDir::new().unwrap();
        let docx_path = temp_dir.path().join("test.docx");

        // Create a dummy file
        std::fs::write(&docx_path, b"dummy content").unwrap();

        // Load with mock converter
        let converter = MockConverter {
            should_succeed: true,
        };
        let result = load_context_file(&docx_path, temp_dir.path(), Some(&converter));

        assert!(result.is_ok());
    }

    #[test]
    fn test_load_context_file_converter_failure() {
        let temp_dir = TempDir::new().unwrap();
        let docx_path = temp_dir.path().join("test.docx");

        // Create a dummy file
        std::fs::write(&docx_path, b"dummy content").unwrap();

        // Load with failing converter
        let converter = MockConverter {
            should_succeed: false,
        };
        let result = load_context_file(&docx_path, temp_dir.path(), Some(&converter));

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), RenderError::OfficeError(_)));
    }

    #[test]
    fn test_load_context_file_pdf_case_insensitive() {
        let temp_dir = TempDir::new().unwrap();
        let pdf_path = temp_dir.path().join("test.PDF"); // uppercase extension

        // Create and save a test PDF
        let mut doc = create_test_pdf();
        doc.save(&pdf_path).unwrap();

        // Should recognize .PDF as PDF
        let result = load_context_file(&pdf_path, temp_dir.path(), None::<&MockConverter>);
        assert!(result.is_ok());
    }

    // ===================
    // merge_pdfs tests
    // ===================

    #[test]
    fn test_merge_pdfs_no_context() {
        let findings = create_test_pdf();
        let context_docs = vec![];

        let result = merge_pdfs(findings, context_docs);
        assert!(result.is_ok());

        let merged = result.unwrap();
        assert_eq!(merged.get_pages().len(), 1);
    }

    #[test]
    fn test_merge_pdfs_with_prepend() {
        let findings = create_test_pdf();
        let prepend_doc = create_test_pdf();

        let context_docs = vec![(prepend_doc, ContextPosition::Prepend)];

        let result = merge_pdfs(findings, context_docs);
        assert!(result.is_ok());

        let merged = result.unwrap();
        assert_eq!(merged.get_pages().len(), 2);
    }

    #[test]
    fn test_merge_pdfs_with_append() {
        let findings = create_test_pdf();
        let append_doc = create_test_pdf();

        let context_docs = vec![(append_doc, ContextPosition::Append)];

        let result = merge_pdfs(findings, context_docs);
        assert!(result.is_ok());

        let merged = result.unwrap();
        assert_eq!(merged.get_pages().len(), 2);
    }

    #[test]
    fn test_merge_pdfs_prepend_and_append() {
        let findings = create_test_pdf();
        let prepend_doc = create_test_pdf();
        let append_doc = create_test_pdf();

        let context_docs = vec![
            (prepend_doc, ContextPosition::Prepend),
            (append_doc, ContextPosition::Append),
        ];

        let result = merge_pdfs(findings, context_docs);
        assert!(result.is_ok());

        let merged = result.unwrap();
        assert_eq!(merged.get_pages().len(), 3);
    }

    #[test]
    fn test_merge_pdfs_multiple_prepend() {
        let findings = create_test_pdf();
        let prepend1 = create_test_pdf();
        let prepend2 = create_test_pdf();

        let context_docs = vec![
            (prepend1, ContextPosition::Prepend),
            (prepend2, ContextPosition::Prepend),
        ];

        let result = merge_pdfs(findings, context_docs);
        assert!(result.is_ok());

        let merged = result.unwrap();
        assert_eq!(merged.get_pages().len(), 3);
    }

    #[test]
    fn test_merge_pdfs_output_is_valid() {
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("merged.pdf");

        let findings = create_test_pdf();
        let prepend_doc = create_test_pdf();

        let context_docs = vec![(prepend_doc, ContextPosition::Prepend)];

        let mut merged = merge_pdfs(findings, context_docs).unwrap();

        // Save and reload to verify it's a valid PDF
        merged.save(&output_path).unwrap();
        let reloaded = Document::load(&output_path);
        assert!(reloaded.is_ok());
    }
}
