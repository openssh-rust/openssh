use openssh::{KnownHosts, Session};

pub fn addr() -> String {
    std::env::var("TEST_HOST").unwrap_or("ssh://test-user@127.0.0.1:2222".to_string())
}

pub async fn connects() -> Vec<Session> {
    let mut sessions = Vec::with_capacity(2);

    #[cfg(feature = "process-mux")]
    {
        sessions.push(Session::connect(&addr(), KnownHosts::Accept).await.unwrap());
    }

    #[cfg(feature = "native-mux")]
    {
        sessions.push(
            Session::connect_mux(&addr(), KnownHosts::Accept)
                .await
                .unwrap(),
        );
    }

    sessions
}
