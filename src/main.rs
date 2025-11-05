use anyhow::{Context, Result};
use ftdc_importer::{
    prometheus::PrometheusRemoteWriteClient, reader::FtdcReader,
    victoria_metrics::VictoriaMetricsClient, ImportMetadata,
};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
#[structopt(
    name = "ftdc-importer",
    about = "Import FTDC files into Victoria Metrics"
)]
struct Opt {
    /// Input FTDC file path
    #[structopt(parse(from_os_str))]
    input: PathBuf,

    /// Victoria Metrics URL (e.g., <http://localhost:8428>)
    #[structopt(long, default_value = "http://localhost:8428")]
    vm_url: String,

    /// Verbose output
    #[structopt(short, long)]
    verbose: bool,

    /// Check mode - analyze all metrics in the file without sending
    #[structopt(long)]
    check: bool,

    /// Clean up existing metrics for this specific file before importing
    #[structopt(long)]
    clean: bool,

    /// Extra label to add to all metrics (format: name=value)
    #[structopt(long, number_of_values = 1, multiple = true)]
    extra_label: Vec<String>,

    /// Dump mode - output parsed metrics as JSON Lines to stdout instead of sending to server
    #[structopt(long)]
    dump: bool,

    /// Filter output to a specific metric name (only works with --dump)
    #[structopt(long)]
    metric: Option<String>,
}

/// Run the check mode to analyze FTDC file contents without sending to Victoria Metrics
async fn run_check_mode(reader: &mut FtdcReader) -> Result<()> {
    println!("CHECK MODE: Analyzing all metrics in the file");

    let mut document_count = 0;
    let mut total_metric_count = 0;
    let mut metric_examples = HashMap::new();

    // Process all documents using time series format
    while let Some(doc) = reader
        .read_next_time_series()
        .await
        .context("Failed to read FTDC document in time series format")?
    {
        document_count += 1;
        total_metric_count += doc.metrics.len();

        // Collect statistics
        for metric in &doc.metrics {
            // Store metric example (metrics are guaranteed to be unique)
            metric_examples.insert(metric.name.clone(), metric.clone());
        }

        // Print progress every 10 documents
        if document_count % 10 == 0 {
            println!("Processed {document_count} documents ({total_metric_count} metrics)");
        }
    }

    // Print statistics
    println!("\n=== FTDC File Analysis (Time Series Format) ===");
    println!("Total documents: {document_count}");
    println!("Total metrics: {total_metric_count}");
    println!("Unique metric names: {}", metric_examples.len());

    // Print examples of each metric
    println!("\nExample metrics:");
    let mut sorted_examples: Vec<_> = metric_examples.values().collect();
    sorted_examples.sort_by(|a, b| a.name.cmp(&b.name));

    for (i, metric) in sorted_examples.iter().enumerate() {
        if i < 10 {
            // Limit to first 10 examples to avoid verbose output
            println!("\nMetric example {}:", i + 1);
            println!("  Name: {}", metric.name);
            println!("  Sample count: {}", metric.values.len());
            if !metric.values.is_empty() {
                // FtdcTimeSeries appears to only have name and values fields
                println!("  First value: {:?}", metric.values.first());
            }
        }
    }

    // Print all metric names
    println!("\nAll metric names ({} total):", metric_examples.len());
    let mut sorted_metrics: Vec<_> = metric_examples.keys().collect();
    sorted_metrics.sort(); // Sort alphabetically for easier reading

    for (i, name) in sorted_metrics.iter().enumerate() {
        println!("{}. {}", i + 1, name);
    }

    Ok(())
}

/// Run dump mode to output parsed metrics as JSON Lines to stdout
async fn run_dump_mode(reader: &mut FtdcReader, metric_filter: Option<&str>) -> Result<()> {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();

    // Process all documents using time series format
    while let Some(doc) = reader
        .read_next_time_series()
        .await
        .context("Failed to read FTDC document in time series format")?
    {
        // Each metric gets its own set of data points
        for metric in &doc.metrics {
            // Apply metric filter if specified
            if let Some(filter) = metric_filter {
                if metric.name != filter {
                    continue;
                }
            }

            // For each timestamp, create a JSON object
            for (i, (&value, timestamp)) in metric.values.iter().zip(&doc.timestamps).enumerate() {
                // Convert SystemTime to Unix timestamp (milliseconds)
                let timestamp_ms = timestamp
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);

                // Create a JSON object for this data point
                let json_line = serde_json::json!({
                    "metric": metric.name,
                    "timestamp": timestamp_ms,
                    "value": value,
                    "sample_index": i,
                });

                // Write as JSON Lines (one JSON object per line)
                writeln!(handle, "{}", json_line)?;
            }
        }
    }

    Ok(())
}

/// Import FTDC metrics to a Prometheus compatible endpoint via remote write API
async fn run_import_mode_prometheus(
    reader: &mut FtdcReader,
    client: &PrometheusRemoteWriteClient,
) -> Result<(usize, usize)> {
    let mut document_count = 0;
    let mut metric_count = 0;

    // Process all documents in time series format
    while let Some(doc) = reader.read_next_time_series().await? {
        document_count += 1;
        metric_count += doc.metrics.len();

        let sample_count = doc.metrics.first().map_or(0, |m| m.values.len());

        // print progress
        println!(
            "Sent total {document_count} documents ({metric_count} metrics); last chunk had {sample_count} samples"
        );

        // Import the document via Prometheus remote write
        client.import_document_ts(&doc).await?;
    }

    Ok((document_count, metric_count))
}

#[tokio::main]
async fn main() -> Result<()> {
    let opt = Opt::from_args();

    if opt.verbose {
        println!("Importing FTDC file: {:?}", opt.input);
        println!("Victoria Metrics URL: {}", opt.vm_url);
    }

    let start = Instant::now();

    let mut reader = FtdcReader::new(&opt.input)
        .await
        .context("Failed to create FTDC reader")?;

    // Clone vm_url before using it
    let vm_url = opt.vm_url.clone();
    let mut metadata = ImportMetadata::default();

    // Parse the extra label strings and add them to metadata
    for label_str in &opt.extra_label {
        if let Some(pos) = label_str.find('=') {
            let name = label_str[..pos].trim().to_string();
            let value = label_str[pos + 1..].trim().to_string();
            if name.is_empty() {
                println!("Warning: Invalid label format (empty name): {label_str}");
            } else {
                if opt.verbose {
                    println!("Adding extra label: {name}={value}");
                }
                metadata.add_extra_label(name, value);
            }
        } else {
            println!("Warning: Invalid label format (missing '='): {label_str}");
        }
    }

    let client = VictoriaMetricsClient::new(&opt.vm_url, metadata.clone());

    if opt.check {
        // Run in check mode without sending metrics
        return run_check_mode(&mut reader).await;
    }

    if opt.dump {
        // Run in dump mode to output metrics as JSON Lines to stdout
        return run_dump_mode(&mut reader, opt.metric.as_deref()).await;
    }

    // Clean up old metrics only if clean flag is specified
    if opt.clean {
        client.cleanup_old_metrics(opt.verbose).await?;
    }

    // Create the Prometheus Remote Write client
    let prometheus_url = format!("{vm_url}/api/v1/write");
    let prom_client = PrometheusRemoteWriteClient::new(prometheus_url, metadata);
    // Use time series format for Victoria Metrics
    println!("Using time series format for Victoria Metrics");
    let result = run_import_mode_prometheus(&mut reader, &prom_client).await?;

    // Use regular format for Victoria Metrics
    // run_import_mode(&mut reader, &client, opt.verbose).await?

    let (document_count, metric_count) = result;
    let elapsed = start.elapsed();

    println!("Import completed successfully!");
    println!("Processed {document_count} documents with {metric_count} metrics in {elapsed:.2?}");
    println!(
        "Average processing speed: {:.2} documents/sec",
        document_count as f64 / elapsed.as_secs_f64()
    );

    Ok(())
}
