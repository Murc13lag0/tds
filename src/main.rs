use chrono::{DateTime, Utc};
use dotenv::dotenv;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();

    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: tds <from> <to>");
        std::process::exit(1);
    }

    let from = &args[1];
    let to = &args[2];
    let api_key = env::var("ORS_API_KEY")?;

    let (transport_dur, drive_dur) = tokio::join!(
        transfer_duration_rail(from, to),
        car_duration(from, to, &api_key)
    );

    match transport_dur {
        Ok(val) => println!("Optimal travel time by train: {val}\n"),
        Err(e) => eprintln!("Train travel error: {e}\n"),
    };

    match drive_dur {
        Ok(val) => println!("Estimated travel time by vehicle: {val}"),
        Err(e) => eprintln!("Car travel error: {e}"),
    };

    Ok(())
}

async fn car_duration(
    from: &str,
    to: &str,
    api_key: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();

    // geocode addresses

    let from_coord = geo(&client, from, api_key).await?;
    let to_coord = geo(&client, to, api_key).await?;

    // get driving duration
    let url = "https://api.openrouteservice.org/v2/directions/driving-car";
    let body = serde_json::json!({
        "coordinates": [[from_coord.0, from_coord.1], [to_coord.0, to_coord.1]]
    });

    let res: serde_json::Value = client
        .post(url)
        .header("Authorization", api_key)
        .json(&body)
        .send()
        .await?
        .json()
        .await?;

    let seconds = match res["routes"][0]["summary"]["duration"].as_f64() {
        Some(v) => v,
        None => return Err("Missing or invalid duration".into()),
    };

    let minutes = (seconds / 60.0).round();

    Ok(format!("{minutes} min"))
}

async fn transfer_duration_rail(
    from: &str,
    to: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let v: serde_json::Value = reqwest::Client::new()
        .get("https://transport.opendata.ch/v1/connections")
        .query(&[("from", from), ("to", to), ("limit", "5")])
        .send()
        .await?
        .json()
        .await?;

    let now = Utc::now().naive_utc();

    let connections = v["connections"].as_array().ok_or("Invalid API response")?;
    let best_conn = connections
        .iter()
        .min_by_key(|c| {
            let dep = DateTime::parse_from_str(
                c["from"]["departure"].as_str().unwrap(),
                "%Y-%m-%dT%H:%M:%S%z",
            )
            .unwrap()
            .naive_utc();

            let arr = DateTime::parse_from_str(
                c["to"]["arrival"].as_str().unwrap(),
                "%Y-%m-%dT%H:%M:%S%z",
            )
            .unwrap()
            .naive_utc();

            (dep - now).num_minutes().max(0) + (arr - dep).num_minutes()
        })
        .ok_or("No connections found")?;

    let duration_str = best_conn["duration"].as_str().unwrap_or("?");
    let transfers = best_conn["transfers"].as_i64().unwrap_or(0);
    let dur_min = parse_duration_to_minutes(duration_str).unwrap_or(0);

    let mut out = Vec::new();
    out.push(format!("{dur_min} min | Transfers: {transfers}"));

    for section in best_conn["sections"].as_array().unwrap() {
        let dep_str = match section["departure"]["departure"].as_str() {
            Some(s) => s,
            None => continue,
        };
        let arr_str = match section["arrival"]["arrival"].as_str() {
            Some(s) => s,
            None => continue,
        };

        let dep_time = &dep_str[11..16];
        let arr_time = &arr_str[11..16];

        let from = section["departure"]["station"]["name"]
            .as_str()
            .unwrap_or("?");
        let to = section["arrival"]["station"]["name"]
            .as_str()
            .unwrap_or("?");

        if section["journey"].is_null() {
            out.push(format!("{dep_time}-{arr_time} | walk → {to}"));
        } else {
            let category = section["journey"]["category"].as_str().unwrap_or_default();
            let number = section["journey"]["number"].as_str().unwrap_or_default();
            let line = format!("{category} {number}").trim().to_string();
            let platform = section["departure"]["platform"].as_str().unwrap_or("-");
            out.push(format!(
                "{dep_time}-{arr_time} | Line: {line} | via [{from}] → [{to}] | Platform: {platform}"
            ));
        }
    }

    Ok(out.join("\n"))
}

fn parse_duration_to_minutes(s: &str) -> Option<u32> {
    let parts: Vec<&str> = s.split('d').last()?.split(':').collect();
    if parts.len() != 3 {
        return None;
    }

    let hours: u32 = parts[0].parse().ok()?;
    let minutes: u32 = parts[1].parse().ok()?;

    Some(hours * 60 + minutes)
}

async fn geo(
    client: &reqwest::Client,
    place: &str,
    api_key: &str,
) -> Result<(f64, f64), Box<dyn std::error::Error>> {
    let url = "https://api.openrouteservice.org/geocode/search";
    let res: serde_json::Value = client
        .get(url)
        .query(&[("api_key", api_key), ("text", place)])
        .send()
        .await?
        .json()
        .await?;

    let coord = &res["features"][0]["geometry"]["coordinates"];
    Ok((coord[0].as_f64().unwrap(), coord[1].as_f64().unwrap()))
}
