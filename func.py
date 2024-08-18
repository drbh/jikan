import requests

# URL of the JSONPlaceholder API
url = "https://jsonplaceholder.typicode.com/posts"

# Sending a GET request to the API
response = requests.get(url)

# Checking if the request was successful
if response.status_code == 200:
    # Parsing the response JSON content
    data = response.json()
    # Printing the first post
    print("First post:")
    print(f"Title: {data[0]['title']}")
    print(f"Body: {data[0]['body']}")
else:
    print(f"Failed to retrieve data. HTTP Status Code: {response.status_code}")
