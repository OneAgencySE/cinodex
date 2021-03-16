use std::sync::Arc;

use crate::client::API_PATH;
use crate::CINodeClient;
use regex::Regex;
use reqwest::Response;
use serde::{Deserialize, Serialize};
use tokio::{
    fs::{create_dir_all, remove_file, File},
    io::AsyncWriteExt,
    sync::mpsc,
};

lazy_static! {
    static ref REGEX: Regex = Regex::new("filename=.*;").unwrap();
}

#[derive(Clone)] // The inner values are not worth an arc, the client itself is wrapped in an ARC internally
pub struct FileExtractor {
    client: CINodeClient,
}
#[derive(Deserialize, Serialize, Debug)]
struct Customer {
    name: String,
    id: u32,
}

#[derive(Deserialize, Serialize, Debug, Default)]
struct CustomerDetails {
    id: Option<u32>,
    projects: Vec<Project>,
    attachments: Vec<Attachment>,
}

#[derive(Deserialize, Serialize, Debug)]
struct Attachment {
    id: String,
    title: String,
    // description: String,
    #[serde(rename = "attachmentType")]
    attachment_type: u32, // File=0, Uri=1
    links: Option<Vec<Link>>,
}

#[derive(Deserialize, Serialize, Debug)]
struct Link {
    href: String,
    rel: Option<String>,
    methods: Vec<String>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
struct Project {
    id: u32,
    title: String,
}

#[derive(Deserialize, Serialize, Debug)]
struct ProjectDetailed {
    id: u32,
    title: String,
    #[serde(rename = "customerId")]
    customer_id: u32,
}

#[derive(Deserialize, Serialize, Debug, Default)]
struct ProjectAttachments {
    attachments: Vec<Attachment>,
}

#[derive(Deserialize, Serialize, Debug)]
struct SubContractor {
    id: u32,
    #[serde(rename = "companyUserId")]
    company_user_id: u32,
    #[serde(rename = "firstName")]
    first_name: String,
    #[serde(rename = "lastName")]
    last_name: String,
    attachments: Vec<SubContractorAttachment>,
}

#[derive(Deserialize, Serialize, Debug)]
struct SubContractorAttachment {
    id: String,
}

impl FileExtractor {
    pub fn new(client: CINodeClient) -> Self {
        FileExtractor { client }
    }

    pub async fn extract(self) {
        println!("Downloading customer files...");
        &self.extract_customer_files().await;
        println!("Done downloading customer files...");

        println!("Downloading sub contractor files...");
        &self.extract_sub_contractor_attachments().await;
        println!("Done downloading sub contractor files...");

        println!("Done!");
    }

    async fn extract_sub_contractor_attachments(&self) {
        let folder = "One Agency/_Sub Contractors";
        let sub_contractors = self
            .client
            .get_cached::<Vec<SubContractor>>(format!("{}/subcontractors", API_PATH))
            .await;

        for contractor in sub_contractors {
            let client = self.client.clone();
            tokio::spawn(async move {
                if contractor.attachments.len() != 0
                    && !file_count_match(&folder, contractor.attachments.len()).await
                {
                    return;
                }

                let folder = format!(
                    "{}/{} {}",
                    folder, contractor.first_name, contractor.last_name
                );

                for attachment in &contractor.attachments {
                    // A sub contractor can only have files, not URIs
                    let response = client
                        .inner
                        .get(&format!(
                            "{}/subcontractors/{}/attachments/{}",
                            API_PATH, contractor.id, attachment.id
                        ))
                        .send()
                        .await
                        .unwrap();
                    print_file_response(response, &folder).await;
                }
            });
        }
    }

    async fn extract_customer_files(&self) {
        let customers = self
            .client
            .get_cached::<Vec<Customer>>(format!("{}/customers", API_PATH))
            .await;

        let (tx, mut rx) = mpsc::channel::<(u32, Vec<Project>)>(100);
        let tx = Arc::new(tx);

        for customer in customers {
            let client = self.client.clone();
            let tx = Arc::clone(&tx);
            tokio::spawn(async move {
                let customer_details = client
                    .get_cached::<CustomerDetails>(format!(
                        "{}/customers/{}",
                        API_PATH, customer.id
                    ))
                    .await;

                if customer_details.attachments.len() == 0 && customer_details.projects.len() == 0 {
                    return;
                }

                let customer_dir = format!("One Agency/{}", customer.name);
                tx.send((
                    customer_details.id.unwrap(),
                    customer_details.projects.clone(),
                ))
                .await
                .unwrap();

                print_customer_attachments(&client, &customer_details, &customer_dir).await;
                print_project_attachments(&client, &customer_details.projects, &customer_dir).await;
            });
        }

        let mut all_projects = self
            .client
            .get_cached::<Vec<ProjectDetailed>>(format!("{}/projects", API_PATH))
            .await;

        while let Some((id, projects)) = rx.recv().await {
            // Remove all projects that matches customer_id && project_id
            // As long as it does not match, keep it
            all_projects.retain(|x| {
                x.customer_id != id && projects.iter().find(|p| p.id == x.id).is_none()
            });
        }

        if all_projects.len() > 0 {
            let folder = "One Agency/_In House Projects".into();
            let proj: Vec<Project> = all_projects
                .iter()
                .map(|x| Project {
                    id: x.customer_id,
                    title: x.title.clone(),
                })
                .collect();
            print_project_attachments(&&self.client.clone(), &proj, &folder).await;
        }
    }
}

async fn print_to_txt(response: Response, folder: &str) {
    let mut text = format!("{}\n{:?}", folder, response);
    if let Ok(bytes) = response.bytes().await {
        if bytes.len() > 0 {
            text.push_str("\n");
            text.push_str(
                &String::from_utf8(bytes.to_vec()).unwrap_or("Could not print bytes".to_string()),
            )
        }
    }
    let file_path = format!("{}/{}.txt", folder, uuid::Uuid::new_v4());
    let _ = create_dir_all(folder).await;
    let mut file = tokio::fs::File::create(&file_path).await.unwrap();
    file.write_all(text.as_bytes()).await.unwrap();
    file.flush().await.unwrap();
}

async fn file_count_match(folder: &str, count: usize) -> bool {
    let mut folder_count = 0;
    if let Ok(mut dir) = tokio::fs::read_dir(folder).await {
        while let Ok(entry) = dir.next_entry().await {
            if let Some(d) = entry {
                if d.path().is_file() {
                    folder_count += 1;
                }
            } else {
                break;
            }
        }
    }
    folder_count == count
}

async fn print_customer_attachments(
    client: &CINodeClient,
    customer: &CustomerDetails,
    folder: &String,
) {
    if file_count_match(&folder, customer.attachments.len()).await {
        return;
    }

    for attachment in &customer.attachments {
        let response = client
            .inner
            .get(&format!(
                "{}/customers/{}/attachments/{}",
                API_PATH,
                customer.id.unwrap(),
                attachment.id
            ))
            .send()
            .await
            .unwrap();

        if attachment.attachment_type == 0 {
            // file
            print_file_response(response, folder).await;
        } else {
            print_to_txt(response, &folder).await;
        }
    }
}

async fn print_project_attachments(
    client: &CINodeClient,
    projects: &Vec<Project>,
    folder: &String,
) {
    for project in projects {
        let project_attachments = client
            .get_cached::<ProjectAttachments>(format!("{}/projects/{}", API_PATH, project.id))
            .await;

        if project_attachments.attachments.len() == 0 {
            continue;
        }

        let title: String = project.title.chars().take_while(|x| x != &'.').collect(); // Some project names contains sentences
        let folder = format!("{}/{}", folder, title);

        if file_count_match(&folder, project_attachments.attachments.len()).await {
            continue;
        }

        for attachment in project_attachments.attachments {
            let response = client
                .inner
                .get(&format!(
                    "{}/projects/{}/attachments/{}",
                    API_PATH, project.id, attachment.id
                ))
                .send()
                .await
                .unwrap();

            if attachment.attachment_type == 0 {
                // file
                print_file_response(response, &folder).await;
            } else {
                print_to_txt(response, &folder).await;
            }
        }
    }
}

async fn print_file_response(response: Response, folder: &String) {
    if response.status().as_u16() == 429 {
        panic!("Too many requests for 24 hrs.")
    }

    let file_name = if let Some(header) = response.headers().get("content-disposition") {
        let string_header = String::from_utf8(header.as_bytes().clone().into()).unwrap();
        let captures = &REGEX.captures(&string_header).unwrap();
        let mut file_name = captures[0]
            .replace("filename=", "")
            .replace(";", "")
            .replace("\"", "");

        if !file_name.contains(".") {
            file_name.push_str(".pdf")
        }

        file_name
    } else {
        let name = format!("{}.txt", uuid::Uuid::new_v4().to_string());
        println!("Print file - Dir: {} - NewName: {}", &folder, &name);
        name
    };

    let mut bytes = response.bytes().await.unwrap();

    if bytes.len() > 0 {
        let file_path = format!("{}/{}", &folder, file_name);
        let _ = create_dir_all(&folder).await; // make sure folder exist
        let _ = remove_file(&file_path).await; // make sure the file is deleted

        let mut file = File::create(&file_path)
            .await
            .expect(&format!("File creation error: {}", file_path));
        file.write_all(&mut bytes).await.unwrap();
        file.flush().await.unwrap();
    }
}
