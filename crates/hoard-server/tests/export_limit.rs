//! The daily export cap, against a real Postgres.
//!
//! Skipped unless `HOARD_PG_TEST_URL` is set, like `orphaned_cursor`:
//!
//! ```sh
//! docker run -d --name hoard-pg -p 55432:5432 \
//!   -e POSTGRES_PASSWORD=hoard -e POSTGRES_DB=hoard postgres:17
//! export HOARD_PG_TEST_URL=postgres://postgres:hoard@localhost:55432/hoard
//! cargo test -p hoard-server --features cloud --test export_limit
//! ```
//!
//! **Never point it at production.**

#![cfg(feature = "cloud")]

use hoard_server::cloud::export::{export_cap_wait, EXPORTS_PER_DAY};
use sqlx::PgPool;
use uuid::Uuid;

async fn pool() -> Option<PgPool> {
    let url = std::env::var("HOARD_PG_TEST_URL").ok()?;
    let pool = hoard_server::cloud::db::connect(&url, 5)
        .await
        .expect("connect to the test database");
    let mut guard = pool.acquire().await.expect("bootstrap connection");
    sqlx::query("SELECT pg_advisory_lock(8_233_119_404)")
        .execute(&mut *guard)
        .await
        .expect("bootstrap lock");
    for role in ["anon", "authenticated", "service_role"] {
        let _ = sqlx::query(&format!("CREATE ROLE {role} NOLOGIN"))
            .execute(&pool)
            .await;
    }
    sqlx::query("CREATE SCHEMA IF NOT EXISTS auth")
        .execute(&pool)
        .await
        .expect("auth schema");
    sqlx::query("CREATE TABLE IF NOT EXISTS auth.users (id UUID PRIMARY KEY)")
        .execute(&pool)
        .await
        .expect("auth.users");
    sqlx::query(
        "CREATE OR REPLACE FUNCTION auth.uid() RETURNS UUID LANGUAGE sql STABLE AS $$ SELECT NULL::uuid $$",
    )
    .execute(&pool)
    .await
    .expect("auth.uid()");
    hoard_server::cloud::db::run_migrations(&pool)
        .await
        .expect("migrations");
    sqlx::query("SELECT pg_advisory_unlock(8_233_119_404)")
        .execute(&mut *guard)
        .await
        .expect("bootstrap unlock");
    drop(guard);
    Some(pool)
}

async fn seed_user(pool: &PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO auth.users (id) VALUES ($1) ON CONFLICT DO NOTHING")
        .bind(id)
        .execute(pool)
        .await
        .expect("auth user");
    sqlx::query("INSERT INTO profiles (user_id, email) VALUES ($1, $2)")
        .bind(id)
        .bind(format!("{id}@test.invalid"))
        .execute(pool)
        .await
        .expect("profile");
    id
}

/// A job requested `hours_ago` hours back.
async fn job(pool: &PgPool, user: Uuid, status: &str, hours_ago: f64) {
    sqlx::query(
        "INSERT INTO export_jobs (user_id, status, requested_at)
         VALUES ($1, $2, now() - make_interval(secs => $3))",
    )
    .bind(user)
    .bind(status)
    .bind(hours_ago * 3600.0)
    .execute(pool)
    .await
    .expect("seed export job");
}

async fn cleanup(pool: &PgPool, user: Uuid) {
    let _ = sqlx::query("DELETE FROM profiles WHERE user_id = $1")
        .bind(user)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM auth.users WHERE id = $1")
        .bind(user)
        .execute(pool)
        .await;
}

#[tokio::test]
async fn the_third_export_of_the_day_closes_the_window() {
    let Some(pool) = pool().await else { return };
    let user = seed_user(&pool).await;
    assert_eq!(EXPORTS_PER_DAY, 3);

    job(&pool, user, "done", 5.0).await;
    job(&pool, user, "done", 3.0).await;
    assert_eq!(export_cap_wait(&pool, user).await.unwrap(), None);

    job(&pool, user, "expired", 1.0).await;
    let wait = export_cap_wait(&pool, user).await.unwrap().expect("capped");
    // The oldest one leaves the window 19 h from now.
    assert!((19 * 3600 - 5..=19 * 3600 + 5).contains(&wait), "{wait}");
    cleanup(&pool, user).await;
}

#[tokio::test]
async fn failed_and_day_old_jobs_do_not_count() {
    let Some(pool) = pool().await else { return };
    let user = seed_user(&pool).await;

    job(&pool, user, "failed", 1.0).await;
    job(&pool, user, "failed", 2.0).await;
    job(&pool, user, "done", 25.0).await;
    job(&pool, user, "expired", 30.0).await;
    job(&pool, user, "done", 2.0).await;
    job(&pool, user, "done", 1.0).await;
    assert_eq!(export_cap_wait(&pool, user).await.unwrap(), None);
    cleanup(&pool, user).await;
}

/// Accounts that exported more than three times before the cap existed wait
/// until enough of those age out, not just the oldest.
#[tokio::test]
async fn an_overfull_window_waits_for_the_right_job() {
    let Some(pool) = pool().await else { return };
    let user = seed_user(&pool).await;

    for hours_ago in [10.0, 8.0, 6.0, 4.0, 2.0] {
        job(&pool, user, "done", hours_ago).await;
    }
    let wait = export_cap_wait(&pool, user).await.unwrap().expect("capped");
    // Five in the window: three must age out to drop under the cap, so the
    // 6 h-old job decides.
    assert!((18 * 3600 - 5..=18 * 3600 + 5).contains(&wait), "{wait}");
    cleanup(&pool, user).await;
}
